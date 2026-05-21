# Bundled model assets

Put optional preinstalled model files here before packaging:

- `crossencoder/model.onnx`
- `crossencoder/tokenizer.json`
- `ner/model.onnx`
- `ner/tokenizer.json`

Packaged apps copy these bundled read-only resources into the user's app data
directory before loading them. If a file is not bundled, QuickTasks downloads it
on first use and stores it in that same writable app data directory.
