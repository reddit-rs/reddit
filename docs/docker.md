# Docker

Images are published to `ghcr.io/reddit-rs/reddit` for `linux/amd64` and
`linux/arm64`. Pin a release tag for reproducibility:

```sh
mkdir -p output
docker run --rm \
  --user "$(id -u):$(id -g)" \
  -v "$PWD/output:/data" \
  -v "$PWD/cookies.txt:/cookies.txt:ro" \
  ghcr.io/reddit-rs/reddit:0.5.4 pics --offline --cookies /cookies.txt
```

The image sets `REDDIT_OUT_DIR=/data`. All [CLI options](usage.md) work in Docker;
pass multiple targets after the image name to archive several listings.

By default the container uses UID/GID 10001. Running with your host UID/GID, as
above, avoids broadening permissions on the output directory. Ensure the cookies
file is readable by the selected user; mount it read-only.

## Build locally

```sh
docker build -t reddit:local .
```

Replace the published image name in the run command with `reddit:local`.
