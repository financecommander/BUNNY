## Triton Ternary Model Assets

Place `.tern` model files here for on-device inference.

- `threat_classifier.tern` — Ternary threat classification model (Phase 2)

These files are loaded via Rust FFI through `TritonInferenceService`.
In Phase 1 (UI development), inference is mocked in pure Dart.
