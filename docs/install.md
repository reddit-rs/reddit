# Installation

## Native binaries

Download a matching archive from [GitHub Releases](https://github.com/reddit-rs/reddit/releases).
Each release contains `reddit-v<version>-<target>.tar.gz` and `SHA256SUMS`.
Archives contain `reddit` (`reddit.exe` on Windows), a README, and both licenses.
Extract the binary to a directory on your `PATH`; check it with `reddit --version`.

| Operating system | x64 target                 | arm64 target                | Requirements                               |
| ---------------- | -------------------------- | --------------------------- | ------------------------------------------ |
| macOS            | `x86_64-apple-darwin`      | `aarch64-apple-darwin`      | macOS 14+                                  |
| Linux            | `x86_64-unknown-linux-gnu` | `aarch64-unknown-linux-gnu` | glibc 2.35+, libstdc++ runtime             |
| Windows          | `x86_64-pc-windows-msvc`   | `aarch64-pc-windows-msvc`   | Windows 10+; arm64 requires Windows on ARM |

Verify downloads before installing. For example, on Linux:

```sh
sha256sum --ignore-missing --check SHA256SUMS
tar -xzf reddit-v0.5.1-x86_64-unknown-linux-gnu.tar.gz
./reddit --version
```

On macOS, use `shasum -a 256 <archive>`; on Windows, use PowerShell
`Get-FileHash <archive> -Algorithm SHA256`. Compare with the matching entry in
`SHA256SUMS`. Linux musl/Alpine users should use the [Docker image](docker.md).

## Homebrew

```sh
brew install reddit-rs/tap/reddit
brew upgrade reddit-rs/tap/reddit
```

The tap lives in [reddit-rs/homebrew-tap](https://github.com/reddit-rs/homebrew-tap).
Homebrew adds it automatically. Successful releases update the formula with
platform-specific binary URLs and SHA-256 checksums once the tap token is configured.
The initial source formula is a bootstrap until the first binary release finishes.
The original tap in this repository remains available for compatibility.

## npm / npx

```sh
npx @rddt/cli --help
npx @rddt/cli pics --offline --cookies cookies.txt
# Pin a version for reproducibility:
npx @rddt/cli@0.5.1 --version
```

Or install globally with `npm install -g @rddt/cli` and run `rddt`.
Node.js 22+ is required. The launcher installs one matching native optional
dependency, forwards arguments and standard I/O without a shell, and preserves
the native exit status. There are no install scripts or runtime binary downloads.
Do not pass `--omit=optional`.
