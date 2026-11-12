# Provenance of the RAW format specifications

This file records how the specifications in `specs/raw/` were made and what their author did and did not consult. It covers `arw.md` (Sony ARW), the first specification of the series.

## Why this file exists

pandite is MIT-licensed and ships through the Microsoft Store. The open-source raw decoders (LibRaw, dcraw, RawSpeed, rawloader, rawler, darktable, RawTherapee and others) carry LGPL or GPL licenses. File formats are facts, and anyone can implement them. Existing code is not free to copy. pandite's raw decoder must therefore be an independent work, and this file is the evidence that its input is clean.

## Method

The work follows a two-step clean-room process.

1. A specification author reads public documents and measures public sample files, then writes a format specification in prose, tables, formulas and test vectors. The author writes no decoder code for the product.
2. A different implementer writes the decoder from the specification alone, plus the public standards it cites (TIFF 6.0 and ITU-T T.81). The implementer never sees the material the author read.

The author of `arw.md` worked as follows:

- Read the official specifications and standards listed as S1 to S4 and S12 in `arw.md`.
- Read public prose documentation: the ExifTool tag reference tables (S5, S6), the body of one RawDigger article (S7), a Sony help page (S8), DXOMARK measurement pages (S9), one survey article on demosaicing (S10) and one expired patent (S11).
- Downloaded public-domain (CC0) sample files from the archive raw.pixls.us and measured them (S13 to S27). Measurements were hex inspection, statistics, and decoding attempts with small tools that the author wrote for this purpose from TIFF 6.0 and ITU-T T.81 alone. Each tool tested a hypothesis stated in the specification text. The results are the measurement records M1 to M16 in `arw.md`. The tools stay outside the repository and are not part of any deliverable. One comparison decoded the camera's own embedded JPEG previews with the Pillow imaging library, which is a general image library and contains no raw decoder.
- Wrote every algorithm in prose, tables and formulas, with worked numeric examples and hex test vectors that the author computed and checked with the measurement tools.

## Rules the author followed

1. Never open, fetch, search inside, quote or paraphrase the source code of any existing raw decoder or raw-processing program, under any license. This includes repositories, source browsers, code snippets in issue trackers, and code pasted in forum posts or blogs. A page that turns out to contain decoder source code is left at once, is not used, and is listed below under "Encountered and not used".
2. Use only these source kinds: official specifications and standards, vendor publications and patents, public prose documentation of the format, and the author's own measurements of sample files. Public prose documentation means format description pages, file-format wikis, metadata tag reference tables published as documentation, conference papers and articles that describe structure and algorithms in words, tables or formulas.
3. Do not rely on memory of any decoder's code. A detail that could come from memory of an implementation is included only if a permitted public document states it or a recorded measurement establishes it. Otherwise it goes to Open questions.
4. Describe algorithms in prose, tables, formulas and worked numeric examples. No code in any programming language and no pseudo-code that mirrors the structure, names or constant layout of an existing implementation.
5. Record for every source its id, title, URL, kind, access date and the sections that rely on it.

## Allowed source kinds

| Kind | Examples used in arw.md |
| --- | --- |
| official | TIFF 6.0 (S1), DNG Specification 1.6.0.0 (S3) |
| standard | ITU-T T.81 (S2), Exif 2.32 (S4), sRGB (S12) |
| patent | Bayer, US 3,971,065 (S11) |
| vendor | Sony Help Guide, RAW File Type (S8) |
| community documentation | ExifTool tag tables (S5, S6), RawDigger article body (S7), DXOMARK measurement pages (S9), Gunturk et al. survey article (S10) |
| measurement | the author's measurements of CC0 sample files (S13 to S27, records M1 to M16) |

## What was consulted

- The sources S1 to S27 listed in `arw.md`, each for the purpose stated in its row.
- Web search result pages (titles and short summaries only) to find the sources above.
- The gate file `.unlazy/pandite/gates/leaf-1.4.1.md` and the structure oracle `scripts/gates/rawspec.mjs` of this repository, for the required document structure.

## What was not consulted

- No source code of any raw decoder or raw-processing program: not LibRaw, dcraw, RawSpeed, rawloader, rawler, dnglab, darktable, RawTherapee, the Adobe DNG SDK, nor the source modules of ExifTool.
- No decoder code in this repository, in particular not `crates/pandite-codecs/src/raw.rs`.
- No paid standard that the author could not read: ISO 12234-2 (TIFF/EP), IEC 61966-2-1 (sRGB transfer function) and ISO 17321-1 are named in `arw.md` only where a read source cites them.

## Encountered and not used

| Page | What happened | Use |
| --- | --- | --- |
| https://github.com/lclevy/sony_raw (readme.md) | The author opened the readme to look for a format description. Its outline and the first lines of its compression section state that the compression description is "based on dcraw and dnglab code source". The author stopped reading at that statement. | Nothing used. The variants, bit layouts and tags of `arw.md` come from S5 to S8 and from measurements. |
| Comment section below the S7 article (https://www.rawdigger.com/howtouse/sony-craw-arw2-posterization-detection) | Some comments discuss the behavior of dcraw source code. | Not used. Only the article body of S7 is cited. The boundary behavior that one comment discusses (a span of exactly 128) was settled by measurement M3. |
| Search results that pointed to decoder source code: rawkit `arw2.rs` on docs.rs, `dcraw.c` in the DragonFly BSD source browser, LibRaw `sonycc.cpp` on a source mirror, the README of the FLIF `raw-tools/sony_arw` folder (stuff.mit.edu), LibRaw forum threads (libraw.org/node/2448, libraw.org/node/2723) | Seen only as titles and short summaries in search results. | Not opened, not used. |
| ExifTool forum thread "Extract decoded SR2 content from Sony ARW images" | Seen only as a search result title. | Not opened, not used. The SR2SubIFD decryption stays an open question (OQ-1). |
| DXOMARK "Basic Measurements" PDF (corp.dxomark.com) | Opened to find the definition of the color matrix. It contains none. | Nothing used. |

## Statement

The specification `specs/raw/arw.md` contains no code copied or translated from any existing decoder. Every normative statement in it traces to a listed public source or to a recorded measurement of public-domain sample files. Where neither was available (SR2SubIFD decryption, the maker-note substitution cipher, the ARW 1.0 and ARW 2.x variable-length codings, the meaning of several Sony tags), the specification leaves the detail out, names it under Open questions, and gives the safe fallback.

Signed: the specification author (Claude, an AI model acting as specification author for leaf 1.4.1), 2026-11-13.
