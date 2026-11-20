# English word list

`en_US.txt` is embedded in the editor binary with `include_str!` (see
`src/spell.rs`) and is the spell checker's base dictionary: one word per line,
lowercase, sorted, no duplicates.

## Provenance

Derived from the SCOWL `en_US` Hunspell dictionary, release `rel-2026.02.25`
(<https://wordlist.aspell.net/>, <https://github.com/en-wl/wordlist>), which is
distributed under the MIT-like licence in `LICENSE`. `README_en_US.txt` is the
dictionary's own README, kept for reference.

The licence requires the copyright notice to accompany copies of the data and to
appear in supporting documentation, so `LICENSE` ships with the sources and the
notice is repeated in the top-level `README.md`.

## Regenerating

The dictionary uses affix compression, so the embedded list is the *expanded*
word set:

```sh
curl -LO https://github.com/en-wl/wordlist/releases/download/rel-2026.02.25/hunspell-en_US-2026.02.25.zip
unzip hunspell-en_US-2026.02.25.zip
python3 expand.py en_US.aff en_US.dic en_US.txt
```

`expand.py` applies each stem's affix flags with Hunspell's combination rules
(one suffix, one prefix, and prefix/suffix pairs only when the rule is marked as
cross-product). It then folds case, normalises `’` to `'`, and drops entries
containing digits.

Output was checked against the `hunspell` tooling: for random sample stems,
`wordforms` produced no form missing from the list, and of 4000 randomly chosen
list entries `hunspell -l` rejected two.
