export interface ParseOutcome {
  title: string;
  doi: string | null;
  source: string;
  suggested_name: string;
  warnings: string[];
}

export interface CopyOutcome {
  status: "Created" | "IdenticalExists" | "AlreadyNamed";
  new_path: string;
}

export type RowStatus = "queued" | "parsing" | "ready" | "failed" | "copied";

export interface Row {
  id: number;
  path: string;
  name: string;
  status: RowStatus;
  title?: string;
  editTitle: string;
  source?: string;
  doi?: string | null;
  suggestedName?: string;
  warnings: string[];
  error?: string;
  newPath?: string;
  copyLabel?: string;
  copyError?: string;
}