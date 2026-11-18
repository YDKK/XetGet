# XetGet

A simple CLI tool for downloading files directly from Hugging Face using Xet.

## Usage

Download `model.onnx` to the current directory.

```console
xetget https://huggingface.co/hf-internal-testing/tiny-random-bert/blob/main/onnx/model.onnx
xetget "https://huggingface.co/hf-internal-testing/tiny-random-bert/resolve/main/onnx/model.onnx?download=true"
```

### Options

```console
xetget URL -O model.safetensors
xetget URL --force
xetget URL --quiet
xetget --help
```

- `-O, --output PATH`: Specify the output path.
- `-f, --force`: Overwrite an existing file.
- `-q, --quiet`: Suppress progress output.

### Authentication

Set the `HF_TOKEN` environment variable.

PowerShell:

```powershell
$env:HF_TOKEN = 'hf_...'
xetget URL
```

Linux:

```sh
HF_TOKEN=hf_... xetget URL
```

## Installation

Install from source:

```console
cargo install xetget
```

Install a prebuilt binary:

```console
cargo binstall xetget
```

Prebuilt releases cover Windows x64 and Linux x64/Arm64 (glibc 2.39+). Install
[cargo-binstall](https://github.com/cargo-bins/cargo-binstall#installation) first.
Other targets may fall back to a source build.

## License

Apache-2.0
