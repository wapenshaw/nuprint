# Bundled fonts

NuPrint renders text with these fonts only. System fonts are never consulted, so output is
identical on Windows, macOS and Linux. Coverage: Latin (incl. Vietnamese), Greek, Cyrillic.

All files are unhinted TTFs from the Noto project, licensed under the SIL Open Font License 1.1
(see `OFL.txt`, copied from https://github.com/notofonts/latin-greek-cyrillic).

Source: `https://raw.githubusercontent.com/notofonts/notofonts.github.io/4690144b8523e3adfbd34f1a148576f32b95d848/fonts/<Family>/unhinted/ttf/<File>`

| File | Version | Bytes | SHA-256 |
| --- | --- | --- | --- |
| NotoSans-Regular.ttf | 2.015 | 431364 | f3961a9cde016d41a4879aecda1474d3a36d6bf54fa0e4643de029cc2248b0e8 |
| NotoSans-Bold.ttf | 2.015 | 432376 | 87cb2d84472a7d66da659ee47b6cdb9552326e8c128245231f191b6ac72529d9 |
| NotoSansMono-Regular.ttf | 2.014 | 388660 | 87f8ce0522a6c99b743ee5fc75b4073cfdd575639119672828b7b9944b65b4f4 |
| NotoSansMono-Bold.ttf | 2.014 | 387484 | 1100772b2f79c102402a1011df5e2226517d0f40ac553a25fb12fdbad8c11b85 |
| OFL.txt | - | - | cee9892f9f0cc8fe882c9e9537ee6a89621d86ee7ceaf70b02e2b2b1c25c061a |

Changing any font changes rendered bitmaps: bump `RENDERER_VERSION` in `src/lib.rs`.
