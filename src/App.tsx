import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { open } from "@tauri-apps/plugin-dialog";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import type { CopyOutcome, LocalParse, Row, VerifyOutcome } from "./types";

const COPY_LABELS: Record<CopyOutcome["status"], string> = {
  Created: "已生成",
  IdenticalExists: "已存在相同副本",
  AlreadyNamed: "文件名已是标题",
};

let nextId = 1;

function applyVerify(row: Row, out: VerifyOutcome): Row {
  if (row.edited) {
    return { ...row, verified: true, verifying: false };
  }
  return {
    ...row,
    verified: true,
    verifying: false,
    title: out.title,
    editTitle: out.title,
    source: out.source,
    doi: out.doi,
    suggestedName: out.suggested_name,
    warnings: out.warning ? [...row.warnings, out.warning] : row.warnings,
  };
}

export default function App() {
  const [rows, setRows] = useState<Row[]>([]);
  const [crossref, setCrossref] = useState(false);
  const [engine, setEngine] = useState<"loading" | "ready" | "error">("loading");
  const [engineError, setEngineError] = useState("");
  const [dragOver, setDragOver] = useState(false);
  const [copyingAll, setCopyingAll] = useState(false);
  const [tick, setTick] = useState(0);
  const [verifyTick, setVerifyTick] = useState(0);
  const processingRef = useRef(false);
  const rowsRef = useRef<Row[]>([]);
  const verifyingIds = useRef(new Set<number>());

  useEffect(() => {
    rowsRef.current = rows;
  }, [rows]);

  useEffect(() => {
    invoke<string>("init_engine")
      .then(() => setEngine("ready"))
      .catch((e) => {
        setEngine("error");
        setEngineError(String(e));
      });
  }, []);

  useEffect(() => {
    let alive = true;
    invoke<{ crossref: boolean }>("get_settings")
      .then((s) => {
        if (alive) setCrossref(Boolean(s.crossref));
      })
      .catch(() => {});
    return () => {
      alive = false;
    };
  }, []);

  const setCrossrefEnabled = (enabled: boolean) => {
    setCrossref(enabled);
    void invoke("set_crossref", { crossref: enabled }).catch(() => {});
  };

  const addPaths = useCallback(async (paths: string[]) => {
    const collected: string[] = [];
    for (const p of paths) {
      if (/\.pdf$/i.test(p)) {
        collected.push(p);
      } else {
        try {
          const found = await invoke<string[]>("scan_folder", { path: p });
          collected.push(...found);
        } catch {
          // not a directory either — ignore
        }
      }
    }
    if (collected.length === 0) return;
    setRows((prev) => {
      const existing = new Set(prev.map((r) => r.path));
      const added: Row[] = [];
      for (const path of collected) {
        if (existing.has(path)) continue;
        existing.add(path);
        added.push({
          id: nextId++,
          path,
          name: path.replace(/\\/g, "/").split("/").pop() || path,
          status: "queued",
          editTitle: "",
          warnings: [],
        });
      }
      return [...prev, ...added];
    });
  }, []);

  useEffect(() => {
    const unlisten = getCurrentWebview().onDragDropEvent((ev) => {
      if (ev.payload.type === "drop") {
        setDragOver(false);
        void addPaths(ev.payload.paths);
      } else if (ev.payload.type === "enter" || ev.payload.type === "over") {
        setDragOver(true);
      } else {
        setDragOver(false);
      }
    });
    return () => {
      void unlisten.then((fn) => fn());
    };
  }, [addPaths]);

  useEffect(() => {
    if (processingRef.current) return;
    const next = rows.find((r) => r.status === "queued");
    if (!next) return;
    processingRef.current = true;
    setRows((rs) =>
      rs.map((r) => (r.id === next.id ? { ...r, status: "parsing" } : r))
    );
    invoke<LocalParse>("parse_local", { path: next.path })
      .then((out) => {
        setRows((rs) =>
          rs.map((r) =>
            r.id === next.id
              ? {
                  ...r,
                  status: "ready",
                  title: out.title,
                  editTitle: out.title,
                  source: out.source,
                  doi: out.doi,
                  suggestedName: out.suggested_name,
                  warnings: out.warnings,
                  cjk: out.cjk,
                  ext: out.ext,
                  verified: false,
                  error: undefined,
                }
              : r
          )
        );
      })
      .catch((e) => {
        setRows((rs) =>
          rs.map((r) =>
            r.id === next.id
              ? { ...r, status: "failed", error: String(e) }
              : r
          )
        );
      })
      .finally(() => {
        processingRef.current = false;
        setTick((t) => t + 1);
      });
  }, [rows, tick]);

  // Online verification runs while the user can already review/edit titles.
  useEffect(() => {
    if (!crossref) return;
    const limit = 4;
    while (verifyingIds.current.size < limit) {
      const next = rows.find(
        (r) =>
          (r.status === "ready" || r.status === "copied") &&
          !r.verified &&
          !r.edited &&
          !r.cjk &&
          !verifyingIds.current.has(r.id)
      );
      if (!next) return;
      verifyingIds.current.add(next.id);
      patchRow(next.id, { verifying: true });
      invoke<VerifyOutcome>("verify_crossref", {
        title: next.title ?? next.editTitle,
        doi: next.doi ?? null,
        source: next.source ?? "",
        suggested_name: next.suggestedName ?? "",
        cjk: next.cjk ?? false,
        ext: next.ext ?? "pdf",
      })
        .then((out) => {
          setRows((rs) =>
            rs.map((r) => (r.id === next.id ? applyVerify(r, out) : r))
          );
        })
        .catch(() => {
          setRows((rs) =>
            rs.map((r) =>
              r.id === next.id ? { ...r, verified: true, verifying: false } : r
            )
          );
        })
        .finally(() => {
          verifyingIds.current.delete(next.id);
          setVerifyTick((t) => t + 1);
        });
    }
  }, [rows, crossref, verifyTick]);

  const pickFiles = async () => {
    const picked = await open({
      multiple: true,
      filters: [{ name: "PDF", extensions: ["pdf"] }],
    });
    if (picked) await addPaths(Array.isArray(picked) ? picked : [picked]);
  };

  const pickFolder = async () => {
    const picked = await open({ directory: true });
    if (picked) await addPaths([picked]);
  };

  const patchRow = (id: number, patch: Partial<Row>) => {
    setRows((rs) => rs.map((r) => (r.id === id ? { ...r, ...patch } : r)));
  };

  const copyRow = async (row: Row) => {
    const title = row.editTitle.trim();
    if (!title) return;
    patchRow(row.id, { copyError: undefined });
    try {
      const out = await invoke<CopyOutcome>("make_copy", {
        path: row.path,
        title,
      });
      patchRow(row.id, {
        status: "copied",
        newPath: out.new_path,
        copyLabel: COPY_LABELS[out.status],
      });
    } catch (e) {
      patchRow(row.id, { copyError: String(e) });
    }
  };

  const copyAll = async () => {
    setCopyingAll(true);
    try {
      const queue = rowsRef.current
        .filter((r) => r.status === "ready")
        .map((r) => r.id);
      const worker = async () => {
        for (;;) {
          const id = queue.shift();
          if (id === undefined) return;
          const row = rowsRef.current.find((r) => r.id === id);
          if (row && row.status === "ready") await copyRow(row);
        }
      };
      await Promise.all(
        Array.from({ length: Math.min(4, queue.length) }, worker)
      );
    } finally {
      setCopyingAll(false);
    }
  };

  const readyCount = rows.filter((r) => r.status === "ready").length;
  const copiedCount = rows.filter((r) => r.status === "copied").length;
  const failedCount = rows.filter((r) => r.status === "failed").length;

  return (
    <div className={`app ${dragOver ? "drag-over" : ""}`}>
      <header className="topbar">
        <div className="brand">
          <span className="brand-name">PaperRenamer</span>
          <span className="brand-sub">论文 PDF 标题重命名</span>
        </div>
        <div
          className={`engine-badge engine-${engine}`}
          title={engine === "error" ? engineError : undefined}
        >
          {engine === "loading" && "引擎初始化中…"}
          {engine === "ready" && "标题引擎就绪"}
          {engine === "error" && "引擎不可用 · 仅内嵌元数据"}
        </div>
      </header>

      <div className="toolbar">
        <button className="btn" onClick={pickFiles}>选择文件</button>
        <button className="btn" onClick={pickFolder}>选择文件夹</button>
        <button
          className="btn"
          disabled={rows.length === 0}
          onClick={() => setRows([])}
        >
          清空
        </button>
        <label className="check">
          <input
            type="checkbox"
            checked={crossref}
            onChange={(e) => setCrossrefEnabled(e.target.checked)}
          />
          CrossRef 在线校验
        </label>
        <div className="spacer" />
        <button
          className="btn btn-primary"
          disabled={readyCount === 0 || copyingAll}
          onClick={copyAll}
        >
          {copyingAll ? "生成中…" : `全部生成副本${readyCount ? `（${readyCount}）` : ""}`}
        </button>
      </div>

      {rows.length === 0 ? (
        <div className={`dropzone ${dragOver ? "active" : ""}`}>
          <svg width="44" height="44" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.4">
            <path d="M6 2.5h8l4.5 4.5v14a1 1 0 0 1-1 1H6a1 1 0 0 1-1-1v-17a1 1 0 0 1 1-1z" />
            <path d="M14 2.5V7h4.5" />
            <path d="M8.5 12h7M8.5 15.5h7" />
          </svg>
          <div className="dropzone-title">拖拽 PDF 文件或文件夹到窗口</div>
          <div className="dropzone-sub">
            自动解析标题（版面识别 + XMP + CrossRef 交叉验证），确认后在原目录生成重命名副本
          </div>
          <div className="dropzone-sub dim">
            也可以直接把 PDF 拖到 PaperRenamer.exe 图标上，静默批量生成副本
          </div>
        </div>
      ) : (
        <div className="list-wrap">
          <div className="list-hint">继续拖拽 PDF 到窗口可追加文件</div>
          <div className="list-head">
            <span>原文件</span>
            <span>解析标题（可编辑）</span>
            <span>来源</span>
            <span>状态 / 新文件名</span>
            <span>操作</span>
          </div>
          <div className="list-body">
            {rows.map((row) => (
              <div className={`row row-${row.status}`} key={row.id}>
                <div className="cell-name" title={row.path}>
                  {row.name}
                </div>
                <div className="cell-title">
                  {row.status === "ready" || row.status === "copied" ? (
                    <input
                      className="title-input"
                      value={row.editTitle}
                      onChange={(e) =>
                        patchRow(row.id, {
                          editTitle: e.target.value,
                          edited: true,
                        })
                      }
                      spellCheck={false}
                    />
                  ) : row.status === "parsing" ? (
                    <span className="muted">解析中…</span>
                  ) : row.status === "failed" ? (
                    <span className="err-text">{row.error}</span>
                  ) : (
                    <span className="muted">等待中</span>
                  )}
                  {row.warnings.map((w, i) => (
                    <div className="warn-text" key={i}>{w}</div>
                  ))}
                </div>
                <div className="cell-source">
                  {row.source && <span className="badge badge-outline">{row.source}</span>}
                  {row.verifying && <span className="verify-hint">核对中…</span>}
                </div>
                <div className="cell-status">
                  {row.status === "queued" && <span className="badge badge-plain">等待</span>}
                  {row.status === "parsing" && <span className="badge badge-solid">解析中</span>}
                  {row.status === "failed" && <span className="badge badge-solid">失败</span>}
                  {row.status === "ready" && (
                    <>
                      <span className="badge badge-outline">就绪</span>
                      <div className="name-preview" title={row.suggestedName}>
                        {row.suggestedName}
                      </div>
                    </>
                  )}
                  {row.status === "copied" && (
                    <>
                      <span className="badge badge-solid">{row.copyLabel}</span>
                      <div className="name-preview" title={row.newPath}>
                        {row.newPath?.replace(/\\/g, "/").split("/").pop()}
                      </div>
                    </>
                  )}
                  {row.copyError && <div className="err-text">{row.copyError}</div>}
                </div>
                <div className="cell-actions">
                  {row.status === "ready" && (
                    <button className="btn btn-sm" onClick={() => copyRow(row)}>
                      生成副本
                    </button>
                  )}
                  {row.status === "copied" && (
                    <>
                      {row.editTitle.trim() !== row.title && (
                        <button className="btn btn-sm" onClick={() => copyRow(row)}>
                          重新生成
                        </button>
                      )}
                      <button
                        className="btn btn-sm"
                        onClick={() => revealItemInDir(row.newPath || row.path)}
                      >
                        打开目录
                      </button>
                    </>
                  )}
                  {row.status === "failed" && (
                    <button
                      className="btn btn-sm"
                      onClick={() =>
                        patchRow(row.id, { status: "queued", error: undefined })
                      }
                    >
                      重试
                    </button>
                  )}
                  <button
                    className="btn btn-sm btn-ghost"
                    title="从列表移除"
                    onClick={() => setRows((rs) => rs.filter((r) => r.id !== row.id))}
                  >
                    ✕
                  </button>
                </div>
              </div>
            ))}
          </div>
        </div>
      )}

      <footer className="statusbar">
        <span>
          共 {rows.length} 个 · 就绪 {readyCount} · 已生成 {copiedCount} · 失败 {failedCount}
        </span>
        <span className="dim">副本生成在原 PDF 同目录，不改动原文件</span>
      </footer>
    </div>
  );
}