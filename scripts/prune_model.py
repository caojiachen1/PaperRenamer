"""Prune a wapiti CRF model to the observations that carry non-zero weights.

Dropping zero-weight observations does not change any score: they contribute 0
to every label. Flat weight indices are recomputed for the kept observations.
Usage: prune_model.py <in.wapiti> <out.wapiti>
"""
import io
import sys


class Reader:
    def __init__(self, data):
        self.d = data
        self.p = 0

    def line(self):
        i = self.d.find(b'\n', self.p)
        if i < 0:
            i = len(self.d)
        out = self.d[self.p:i]
        self.p = i + 1
        return out

    def netstring(self):
        i = self.d.find(b':', self.p)
        n = int(self.d[self.p:i])
        s = i + 1
        e = s + n
        self.p = e + 2  # skip ',' '\n'
        return self.d[s:e]


def main():
    src, dst = sys.argv[1], sys.argv[2]
    data = io.open(src, 'rb').read()
    r = Reader(data)

    mdl = r.line().decode()
    type_and_nact = mdl.split('#')
    mtype, nact = type_and_nact[2], int(type_and_nact[3])
    rdr = r.line()
    npats = int(rdr.decode().lstrip('#rdr#').split('/')[0])
    pats = [r.netstring() for _ in range(npats)]

    qrk = r.line().decode()
    nlabels = int(qrk.lstrip('#qrk#'))
    labels = [r.netstring() for _ in range(nlabels)]

    qrk = r.line().decode()
    nobs = int(qrk.lstrip('#qrk#'))
    obs = [r.netstring() for _ in range(nobs)]

    kinds = []
    for s in obs:
        c = s[:1]
        kinds.append(1 if c == b'u' else 2 if c == b'b' else 3 if c == b'*' else 0)

    Y = nlabels
    # Old flat offsets, same layout as wapiti mdl_sync.
    old_off = []
    start = 0
    for k in kinds:
        uoff = boff = None
        if k & 1:
            uoff = start
            start += Y
        if k & 2:
            boff = start
            start += Y * Y
        old_off.append((uoff, boff))

    weights = []
    for _ in range(nact):
        line = r.line()
        idx_s, w_s = line.split(b'=')
        weights.append((int(idx_s), float.fromhex(w_s.decode())))

    # Map old flat index -> observation id.
    ranges = []
    for o, (uoff, boff) in enumerate(old_off):
        if uoff is not None:
            ranges.append((uoff, uoff + Y, o, 0))
        if boff is not None:
            ranges.append((boff, boff + Y * Y, o, 1))
    ranges.sort()

    active = {}
    for idx, w in weights:
        lo, hi = 0, len(ranges)
        while lo < hi:
            mid = (lo + hi) // 2
            a, b, o, kind = ranges[mid]
            if idx < a:
                hi = mid
            elif idx >= b:
                lo = mid + 1
            else:
                slot = idx - a
                active.setdefault(o, []).append((slot, w))
                break
        else:
            raise SystemExit(f'weight {idx} outside ranges')

    keep = sorted(active)
    new_id = {o: i for i, o in enumerate(keep)}
    new_off = []
    start = 0
    for o in keep:
        k = kinds[o]
        uoff = boff = None
        if k & 1:
            uoff = start
            start += Y
        if k & 2:
            boff = start
            start += Y * Y
        new_off.append((uoff, boff))

    out = io.open(dst, 'wb')

    def line(s):
        out.write(s if isinstance(s, bytes) else s.encode())

    def ns(s):
        line(b'%d:' % len(s) + s + b',\n')

    nact_new = sum(len(v) for v in active.values())
    line(f'#mdl#{mtype}#{nact_new}\n'.encode())
    line(rdr + b'\n')
    for p in pats:
        ns(p)
    line(f'#qrk#{nlabels}\n'.encode())
    for l in labels:
        ns(l)
    line(f'#qrk#{len(keep)}\n'.encode())
    for o in keep:
        ns(obs[o])

    new_weights = []
    for i, o in enumerate(keep):
        uoff, boff = new_off[i]
        for slot, w in active[o]:
            if uoff is not None and slot < Y:
                new_weights.append((uoff + slot, w))
            else:
                new_weights.append((boff + slot, w))
    new_weights.sort()
    for idx, w in new_weights:
        line(b'%d=%s\n' % (idx, float.hex(w).encode()))

    out.close()
    print(f'labels={nlabels} obs {nobs} -> {len(keep)} weights {nact} -> {nact_new}')


if __name__ == '__main__':
    main()
