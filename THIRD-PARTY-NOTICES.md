# Third-party notices

FileManagerUltra is licensed under the Apache License 2.0 (see [LICENSE](LICENSE)). It is
built with many open-source Rust crates; their licences are listed on crates.io for the
versions pinned in `Cargo.lock`. The parts below have licences that ask to be named with the
program. The app shows the same list, with the full licence texts, in Help ▸ Third-party
Licences.

| Part | Used for | Licence | Source |
|---|---|---|---|
| Atkinson Hyperlegible (Braille Institute of America) | Reading font for Markdown and diagrams | SIL Open Font License 1.1, [text](crates/preview/assets/fonts/OFL.txt) | <https://github.com/google/fonts/tree/main/ofl/atkinsonhyperlegible> |
| Dancing Script (The Dancing Script Project Authors) | Typed visual signatures in PDFs (drawn as outlines, never embedded in saved files) | SIL Open Font License 1.1, [text](crates/pdf/assets/DancingScript-OFL.txt) | <https://github.com/googlefonts/DancingScript> |
| Foxit standard fonts, from PDFium (bundled by the `hayro` PDF renderer) | Drawing text in PDFs that do not embed their fonts | BSD-3-Clause, [text](crates/pdf/licenses/Foxit-fonts-BSD-3-Clause.txt) | <https://pdfium.googlesource.com/pdfium/> |
| Adobe CMap resources (bundled by the `hayro` PDF renderer) | Chinese, Japanese and Korean text in PDFs | BSD-3-Clause, [text](crates/pdf/licenses/Adobe-CMaps-BSD-3-Clause.txt) | <https://github.com/adobe-type-tools/cmap-resources> |
| Eclipse ELK layered layout, Rust port `merman-elk-layered` 0.10.0 | Layout of Mermaid diagrams | Eclipse Public License 2.0, [text](crates/preview/licenses/EPL-2.0.txt) | <https://crates.io/crates/merman-elk-layered/0.10.0> |

The EPL-2.0 code is used unmodified as a separate crate; its source code is available from
the link above. The rest of FileManagerUltra is not covered by the EPL.
