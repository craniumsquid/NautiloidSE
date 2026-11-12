import sys, struct, glob, os, zlib, re
import lz4.block, lz4.frame, zstandard
root = sys.argv[1]
def get(d, tbl, n, want):
    for i in range(n):
        e = tbl[i*272:(i+1)*272]
        name = e[:256].split(b'\0')[0].decode()
        lo, hi, part, fl, sod, us = struct.unpack_from('<IHBBII', e, 256)
        if name == want:
            raw = d[lo|(hi<<32):][:sod]; m = fl & 0xf
            if m == 1: return zlib.decompress(raw)
            if m == 2: return lz4.block.decompress(raw, uncompressed_size=us)
            if m == 3: return zstandard.ZstdDecompressor().decompress(raw, max_output_size=us)
            return raw
for path in sorted(glob.glob(os.path.join(root, '*', '*.lsv'))):
    d = open(path, 'rb').read()
    off = struct.unpack_from('<Q', d, 8)[0]
    n, csize = struct.unpack_from('<II', d, off)
    tbl = lz4.block.decompress(d[off+8:off+8+csize], uncompressed_size=n*272)
    m = get(d, tbl, n, 'meta.lsf')
    sz = struct.unpack_from('<10I', m, 16)
    flags = m[56]
    pos = 64
    secs = []
    for i in range(5):
        us, sod = sz[2*i], sz[2*i+1]
        ln = sod if sod else us
        secs.append((us, sod, m[pos:pos+ln])); pos += ln
    print('==', os.path.basename(os.path.dirname(path)), 'sizes', sz, 'flags', hex(flags), 'consumed', pos, 'of', len(m))
    # try each section order assumption: strings, keys?, nodes, attrs, values
    for idx, (us, sod, raw) in enumerate(secs):
        if not us: continue
        try:
            if sod == 0: out = raw
            elif raw[:4] == b'\x04\x22\x4d\x18': out = lz4.frame.decompress(raw)
            else: out = lz4.block.decompress(raw, uncompressed_size=us)
        except Exception as ex:
            print('  sec', idx, 'fail', ex, raw[:8].hex()); continue
        strs = [s.decode('latin1') for s in re.findall(rb'[\x20-\x7e]{4,}', out)]
        if idx == 0:
            print('  names:', ' '.join(sorted(set(strs)))[:1500])
        else:
            print('  sec', idx, len(out), 'strings:', ' | '.join(strs)[:2500])
