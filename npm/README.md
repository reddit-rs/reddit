# @rddt/cli

A thin JavaScript launcher for the native [reddit](https://github.com/reddit-rs/reddit)
offline archiver. No Rust installation or runtime binary download is required.

```sh
npx @rddt/cli pics --offline --cookies cookies.txt
# Or install the launcher globally:
npm install -g @rddt/cli
rddt --help
```

Supports macOS 14+, Linux with glibc 2.35+, and Windows, on x64 and arm64.
Node.js 22+ is required. Linux musl/Alpine is not supported by this package;
use the project's Docker image instead.

The matching native package is installed as an optional dependency. Do not use
`--omit=optional`. Arguments, standard input/output, and exit status are passed
through to the binary. No shell is used to interpret arguments.

Keep cookie exports private. See the project's [usage guide](https://github.com/reddit-rs/reddit/blob/main/docs/usage.md)
for options and authentication.

MIT OR Apache-2.0. Not affiliated with Reddit, Inc.
