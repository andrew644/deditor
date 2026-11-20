import re, sys, json

aff_path, dic_path, out_path = sys.argv[1:4]

pfx_rules = {}
sfx_rules = {}
pfx_cross = {}
sfx_cross = {}
for line in open(aff_path, encoding="utf-8"):
    parts = line.rstrip("\n").split()
    if not parts or parts[0] not in ("PFX", "SFX"):
        continue
    kind, flag = parts[0], parts[1]
    rules = pfx_rules if kind == "PFX" else sfx_rules
    cross = pfx_cross if kind == "PFX" else sfx_cross
    if len(parts) == 4:
        cross[flag] = parts[2]
    elif len(parts) == 5:
        _, _, strip, add, cond = parts
        pat = None if cond == "." else re.compile(cond + "$" if kind == "SFX" else "^" + cond)
        rules.setdefault(flag, []).append((strip, add, pat))

base = {}
for i, line in enumerate(open(dic_path, encoding="utf-8")):
    if i == 0:
        continue
    w = line.rstrip("\n")
    if not w:
        continue
    word, _, flags = w.partition("/")
    base.setdefault(word, set()).update(flags.split("/")[0])


def apply_sfx(word, flag):
    for strip, add, pat in sfx_rules.get(flag, ()):
        if pat and not pat.search(word):
            continue
        if strip == "0":
            yield word + add
        elif word.endswith(strip) and len(word) > len(strip):
            yield word[: len(word) - len(strip)] + add


def apply_pfx(word, flag):
    for strip, add, pat in pfx_rules.get(flag, ()):
        if pat and not pat.search(word):
            continue
        if strip == "0":
            yield add + word
        elif word.startswith(strip) and len(word) > len(strip):
            yield add + word[len(strip):]


forms = set()
for word, flags in base.items():
    forms.add(word)
    by_sfx = {f: list(apply_sfx(word, f)) for f in flags if f in sfx_rules}
    by_pfx = {g: list(apply_pfx(word, g)) for g in flags if g in pfx_rules}
    for sl in by_sfx.values():
        forms.update(sl)
    for g, pl in by_pfx.items():
        forms.update(pl)
        if pfx_cross.get(g) != "Y":
            continue
        for p in pl:
            for f in flags:
                if f in sfx_rules:
                    forms.update(apply_sfx(p, f))
    for f, sl in by_sfx.items():
        if sfx_cross.get(f) != "Y":
            continue
        for s in sl:
            for g in flags:
                if g in pfx_rules:
                    forms.update(apply_pfx(s, g))

print("rules pfx/sfx:", sum(map(len, pfx_rules.values())), sum(map(len, sfx_rules.values())), file=sys.stderr)
print("base:", len(base), "forms:", len(forms), file=sys.stderr)


def keep(w):
    w = w.replace("\u2019", "'")
    if any(c.isdigit() for c in w):
        return None
    w = w.strip("-'")
    return w or None


final = set()
for w in forms:
    k = keep(w)
    if k:
        final.add(k.lower())

print("final:", len(final), file=sys.stderr)
with open(out_path, "w", encoding="utf-8") as f:
    for w in sorted(final):
        f.write(w + "\n")
json.dump(sorted(forms), open("/tmp/raw_forms.json", "w"))
