# RAW format specifications

This folder holds clean-room specifications of camera raw formats for the pandite decoder.

| File | Content |
| --- | --- |
| `arw.md` | Sony ARW: container, tags, the five storage variants, levels, color, pipeline, preview fallback, error handling, test vectors, open questions |
| `PROVENANCE.md` | How the specifications were made, the source rules, and what was not consulted |

## How to implement a decoder from a specification

1. Read the whole specification before you write code.
2. Use only the specification and the public standards that it cites. For `arw.md` these are TIFF 6.0 (source S1) and ITU-T T.81 (source S2).
3. Do not open the source code of any raw decoder or raw-processing program. Do not search for it. This applies to every license.
4. Implement every numbered requirement (`R-ARW-n`). Each requirement is one testable statement.
5. Turn the test vectors (section "Test vectors") into unit tests. Build the fixtures F1 to F5 from the recipes and check the expected output values.
6. Follow the fallback of each item in "Open questions". Do not guess other behavior.
7. If the specification does not answer a question, stop. Record the question for the specification author. Do not fill the gap from memory of other decoders.
8. Cite the requirement id in a code comment where the code implements it.

## How to check the structure of a specification

Run this command from the repository root:

```
node scripts/gates/rawspec.mjs arw
```

The command must print `RAW SPEC STRUCTURE VERIFIED`. The check covers structure only. The decoder's pixel tests prove the content.
