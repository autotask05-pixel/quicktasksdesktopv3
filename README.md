# QuickTasks Desktop

Tauri desktop app for loading a QuickTasks agent JSON and running local model-backed routing.
https://qiktax.n8271435.workers.dev/

## Development

```bash
cargo tauri dev
```

## Distribution Builds

Build installers with:

```bash
cargo tauri build
```

Tauri builds native installers for the current operating system. For full distribution coverage, run the build on each target platform:

- macOS: builds `.app` and `.dmg` bundles.
- Windows: builds Windows installer artifacts.
- Linux: builds Linux bundle artifacts supported by the host system.

## Bundled Models

The app is configured to package the `models/` directory as a read-only Tauri resource. Before making offline-ready installers, place these files in the repo:

```text
models/crossencoder/model.onnx
models/crossencoder/tokenizer.json
models/ner/model.onnx
models/ner/tokenizer.json
```

At runtime, QuickTasks copies bundled model resources into the user's writable app-data directory before loading them. This avoids permission errors from writing into an installed app bundle. If these files are not bundled, the app downloads the default model files on first model load and stores them in the same app-data directory.

The desktop UI can also point each model asset directly at an existing local file path. In that mode the app validates the path and loads the ONNX or tokenizer JSON from that location without copying the large file into app data.

Environment overrides are still supported for testing and managed installs:

```text
QUICKTASKS_MODEL_DIR
QUICKTASKS_BUNDLED_MODEL_DIR
QUICKTASKS_GTE_MODEL_PATH
QUICKTASKS_GTE_TOKENIZER_PATH
QUICKTASKS_GLINER_MODEL_PATH
QUICKTASKS_GLINER_TOKENIZER_PATH
QUICKTASKS_CONFIG
```
