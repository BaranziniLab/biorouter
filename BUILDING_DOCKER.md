# Building and running Biorouter with Docker

This guide covers the Biorouter CLI, browser server and local dictation in the Docker image.

## Quick Start

### No Pre-built Image — Build Locally

There is **no** published Biorouter container image. The nightly and manual Docker workflow builds and checks an image, but does not push it to a registry. Build from source and use a local tag:

```bash
# Build the image from the repo root
docker build -t biorouter:local .

# Run Biorouter CLI
docker run --rm biorouter:local --version

# Run with LLM configuration
docker run --rm \
  -e BIOROUTER_PROVIDER=openai \
  -e BIOROUTER_MODEL=gpt-5.6 \
  -e OPENAI_API_KEY=$OPENAI_API_KEY \
  biorouter:local run -t "Summarize the latest PubMed review on CRISPR gene therapy"
```

## Building from Source

### Prerequisites

- Docker 20.10 or later
- Docker Buildx (for multi-platform builds)
- Git

### Build the Image

1. Clone the repository:
```bash
git clone https://github.com/BaranziniLab/biorouter.git
cd biorouter
```

2. Build the Docker image:
```bash
docker build -t biorouter:local .
```

The build process:
- Uses a multi-stage build to minimize final image size
- Compiles the CLI and daemon with optimizations and the dictation engine
- Builds the browser bundle in a separate Node 24 stage with `npm ci` and `npm run build:web`
- Copies `biorouter`, `biorouterd`, the browser bundle at `/usr/share/biorouter/web`, and the Biorouter Copilot helper into a Debian runtime image
- Includes no speech model; the person installs one in a persistent data volume

### Build Options

For multi-platform builds, use Buildx. The nightly workflow builds each architecture on a native runner:
```bash
docker buildx build --platform linux/amd64,linux/arm64 \
  --output type=oci,dest=biorouter-multi.tar .
```

## Running Biorouter in Docker

### Browser dictation through `serve`

The container has no microphone. Its `biorouterd` worker transcribes audio recorded by the browser on the computer publishing the port. Keep the port on host loopback so the page is a secure browser context. Open the tokened address printed by the container banner, replacing the host port placeholder with `8765` in this example.

```bash
docker run --rm -p 127.0.0.1:8765:8765 \
  -v biorouter-data:/home/biorouter/.local/share/biorouter \
  -v biorouter-config:/home/biorouter/.config/biorouter \
  biorouter:local serve --host 0.0.0.0
```

Install the Light model and voice detector into the data volume, then restart `serve` with the same volumes. The image stays unchanged:

```bash
docker run --rm \
  -v biorouter-data:/home/biorouter/.local/share/biorouter \
  -v biorouter-config:/home/biorouter/.config/biorouter \
  biorouter:local dictation install --yes whisper-small.en-q8_0
```

For an offline install, mount a folder holding the two files from the pinned Biorouter model releases read only and use `dictation install --yes --from /model-cache`. An administrator can also mount a system model folder read only at `/var/lib/biorouter/dictation/models`. `biorouter transcribe /data/memo.wav` works with a recording mounted at `/data` and the same data volume.

To check a local image with an existing model cache, run the same image checks as CI. The cache folder must hold `ggml-small.en-q8_0.bin` and `ggml-silero-v6.2.0.bin`; the script checks their SHA-256 pins before using them. `--browser` needs the desktop's Playwright package and Chromium on the host. Choose a fresh run folder. The script retains its evidence and named data volumes for inspection.

```bash
bash scripts/check-docker-dictation.sh --image biorouter:local \
  --model-cache /path/to/model-cache --run-root /path/to/new-pr31-run --browser
```

### CLI Mode

Basic usage:
```bash
# Show help
docker run --rm biorouter:local --help

# Run a command
docker run --rm \
  -e BIOROUTER_PROVIDER=openai \
  -e BIOROUTER_MODEL=gpt-5.6 \
  -e OPENAI_API_KEY=$OPENAI_API_KEY \
  biorouter:local run -t "Explain the mechanism of action of metformin"
```

With volume mounts for file access:
```bash
docker run --rm \
  -v $(pwd):/workspace \
  -w /workspace \
  -e BIOROUTER_PROVIDER=openai \
  -e BIOROUTER_MODEL=gpt-5.6 \
  -e OPENAI_API_KEY=$OPENAI_API_KEY \
  biorouter:local run -t "Analyze the patient cohort CSV in this directory and report cohort demographics"
```

Interactive session mode with Databricks:
```bash
docker run -it --rm \
  -e BIOROUTER_PROVIDER=databricks \
  -e BIOROUTER_MODEL=databricks-claude-sonnet-4-6 \
  -e DATABRICKS_HOST="$DATABRICKS_HOST" \
  -e DATABRICKS_TOKEN="$DATABRICKS_TOKEN" \
  biorouter:local session
```



### Docker Compose

Create a `docker-compose.yml`:

```yaml
services:
  biorouter:
    image: biorouter:local
    environment:
      - BIOROUTER_PROVIDER=${BIOROUTER_PROVIDER:-openai}
      - BIOROUTER_MODEL=${BIOROUTER_MODEL:-gpt-5.6}
      - OPENAI_API_KEY=${OPENAI_API_KEY}
    volumes:
      - ./workspace:/workspace
      - biorouter-config:/home/biorouter/.config/biorouter
    working_dir: /workspace
    stdin_open: true
    tty: true

volumes:
  biorouter-config:
```

Run with:
```bash
docker-compose run --rm biorouter session
```

## Configuration

### Environment Variables

The Docker image accepts all standard BioRouter environment variables:

- `BIOROUTER_PROVIDER`: LLM provider (openai, anthropic, google, databricks, etc.)
- `BIOROUTER_MODEL`: Model id to use
- Provider-specific API keys (OPENAI_API_KEY, ANTHROPIC_API_KEY, etc.)

> **Model ids move.** Providers retire endpoints and the built-in catalog tracks them — several ids that appeared in older copies of this guide (`gpt-4o`, `claude-sonnet-4`, `databricks-dbrx-instruct`) no longer resolve. The ids in the examples above are the current per-provider defaults; run `biorouter configure` to see the list your build actually serves rather than trusting any id written down here.

### Persistent Configuration

Mount the configuration directory to persist settings:
```bash
docker run --rm \
  -v ~/.config/biorouter:/home/biorouter/.config/biorouter \
  biorouter:local configure
```

### Installing Additional Tools

The image runs as a non-root user by default. To install additional packages:

```bash
# Run as root to install packages
docker run --rm \
  -u root \
  --entrypoint bash \
  biorouter:local \
  -c "apt-get update && apt-get install -y vim && biorouter --version"

# Or create a custom Dockerfile
FROM biorouter:local
USER root
RUN apt-get update && apt-get install -y \
    vim \
    tmux \
    && rm -rf /var/lib/apt/lists/*
USER biorouter
```

## CI/CD integration

`.github/workflows/docker-image.yml` runs nightly and on manual dispatch. It builds amd64 and arm64 images, checks the offline engine self test and empty model state, installs the SHA-256 pinned Light model into a volume, checks file transcription and `serve`, and drives a fake microphone in Chromium on host loopback. It caches only the model and voice detector after a successful default branch run. The workflow does not publish an image.

A local tag exists only on the Docker host that built it. A separate CI job must build the image from source or load an image artifact before it can run `biorouter:local`. The repository's Docker workflow builds and checks the source in its own jobs. It does not make the tag available to other jobs or registries.

## Image Details

### Size and Optimization

- **Multi-stage build**: the Go, Rust and Node builder stages are discarded; only the compiled artifacts are copied forward
- **Base image**: `debian:bookworm-slim`, pinned by digest, plus eleven runtime packages (`ca-certificates`, `libssl3`, `libdbus-1-3`, `libxcb1`, `python3`, `python3-gi`, `gir1.2-atspi-2.0`, `gir1.2-gtk-3.0`, `at-spi2-core`, `curl`, `git`). The five in the middle are the Biorouter Copilot helper's AT-SPI runtime
- **Optimizations**: the `Dockerfile` overrides the release profile with LTO, one codegen unit, `opt-level=z`, and stripping
- **Contents**: the CLI (`/usr/local/bin/biorouter`), its sibling daemon (`/usr/local/bin/biorouterd`), the web bundle (`/usr/share/biorouter/web`), and the Biorouter Copilot helper (`/usr/libexec/biorouter/computer-use`). No model is stored in the image

Image and binary sizes are not stated here on purpose: they move with the profile settings and the dependency tree, and a stale number is worse than none. Measure your own build with `docker images biorouter:local`.

### Security

- Runs as non-root user `biorouter` (UID 1000)
- Minimal attack surface with only essential runtime dependencies
- **Scheduled checks.** The nightly workflow rebuilds and checks the image. It does not publish or replace a local image

### Included Tools

The image includes tools for Biorouter operation:
- `git` - Version control operations
- `curl` - HTTP requests
- `ca-certificates` - SSL/TLS support
- Basic shell utilities

## Troubleshooting

### Permission Issues

If you encounter permission errors when mounting volumes:
```bash
# Ensure the mounted directory is accessible
docker run --rm \
  -v $(pwd):/workspace \
  -u $(id -u):$(id -g) \
  biorouter:local run -t "List files"
```

### API Key Issues

If API keys aren't being recognized:
1. Ensure environment variables are properly set
2. Check that quotes are handled correctly in your shell
3. Use `docker run --env-file .env` for multiple environment variables

### Network Issues

For accessing local services from within the container:
```bash
# Use host network mode
docker run --rm --network host biorouter:local
```

## Advanced Usage

### Custom Entrypoint

Override the default entrypoint for debugging:
```bash
docker run --rm -it --entrypoint bash biorouter:local
```

### Resource Limits

Set memory and CPU limits:
```bash
docker run --rm \
  --memory="2g" \
  --cpus="2" \
  biorouter:local
```

### Multi-stage Development

For development with hot reload:
```bash
# Mount source code
docker run --rm \
  -v $(pwd):/usr/src/biorouter \
  -w /usr/src/biorouter \
  rust:1.92-bookworm \
  cargo watch -x run
```

## Building for Production

For production deployments:

1. Use specific image tags instead of `latest`
2. Use secrets management for API keys
3. Set up logging and monitoring
4. Configure resource limits and auto-scaling

Example production Dockerfile:
```dockerfile
FROM biorouter:local
# Add any additional tools needed for your use case
USER root
RUN apt-get update && apt-get install -y your-tools && rm -rf /var/lib/apt/lists/*
USER biorouter
```

## Contributing

When contributing Docker-related changes:

1. Test builds on multiple platforms (amd64, arm64)
2. Verify image size remains reasonable
3. Update this documentation
4. Consider CI/CD implications
5. Test with various LLM providers

## Related Documentation

- [Documentation](https://biorouter.ucsf.edu/docs.html) - Installation, configuration, and usage
- [Downloads](https://biorouter.ucsf.edu/download.html) - Prebuilt desktop and CLI packages
