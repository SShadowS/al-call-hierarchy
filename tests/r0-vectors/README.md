# R0 encoder vectors

`encoder-vectors.json` is the differential oracle for the al-sem → Rust identity
encoders. Each vector pairs an `input` with the `expected` string that the REAL
al-sem TypeScript encoder produced (never hand-computed).

- **Provenance**: copied verbatim from al-sem
  `scripts/r0-goldens/encoder-vectors.json` @ commit `f0ae38c`.
- **Refresh**: manual. When al-sem regenerates the vectors, re-copy the file here
  and re-run `cargo test --test encoder_vectors`.
- **Why committed**: so `cargo test` runs fully offline — no Bun, no al-sem
  checkout required.

The Rust ports live in `src/engine/ids.rs`; the test harness is
`tests/encoder_vectors.rs`.
