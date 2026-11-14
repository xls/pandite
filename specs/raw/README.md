# RAW format specifications

This folder holds clean-room specifications of camera raw formats for the pandite decoder.

| File | Content |
| --- | --- |
| `arw.md` | Sony ARW: container, tags, the five storage variants, levels, color, pipeline, preview fallback, error handling, test vectors, open questions |
| `cr2.md` | Canon CR2: lossless JPEG with slices in three frame layouts; sRAW and mRAW out of scope |
| `cr3.md` | Canon CR3: container, metadata and the route to the largest JPEG preview; the CRX sensor codec is out of scope |
| `nef.md` | Nikon NEF: eight storage variants incl. lossless and lossy Huffman coding; High Efficiency out of scope |
| `raf.md` | Fujifilm RAF: three uncompressed layouts, X-Trans and Bayer; compressed files out of scope |
| `orf.md` | Olympus ORF: six storage variants incl. lossless compression at 12 and 14 bits |
| `rw2.md` | Panasonic RW2: eight storage variants (RawFormat 4 to 7, older .RAW); RawFormat 8 out of scope |
| `pef.md` | Pentax PEF: four storage variants incl. Pentax Huffman coding and monochrome |
| `srw.md` | Samsung SRW: two uncompressed layouts; compressed files out of scope |
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
node scripts/gates/rawspec.mjs arw cr2 cr3 nef raf orf rw2 pef srw
```

The command must print `RAW SPEC STRUCTURE VERIFIED`. The check covers structure only. The decoder's pixel tests prove the content.
