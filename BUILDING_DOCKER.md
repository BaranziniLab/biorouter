# Building and Running BioRouter with Docker

This guide covers building Docker images for BioRouter CLI for production use, CI/CD pipelines, and local development.

## Quick Start

### No Pre-built Image — Build Locally

There is **no** published BioRouter container image: no registry hosts one and no CI workflow builds or pushes it. Build the image yourself from source (see [Building from Source](#building-from-source)) and refer to it by its local tag `biorouter:local`:

```bash
# Build the image from the repo root
docker build -t biorouter:local .

# Run BioRouter CLI
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
- Compiles with optimizations (LTO, stripping, size optimization)
- Results in a `debian:bookworm-slim`-based image containing the `biorouter` CLI binary and the native Biorouter Copilot helper (the `biorouterd` daemon is not included)

### Build Options

For multi-platform builds:
```bash
docker buildx build --platform linux/amd64,linux/arm64 -t biorouter:multi .
```

## Running BioRouter in Docker

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

## CI/CD Integration

### GitHub Actions

```yaml
jobs:
  analyze:
    runs-on: ubuntu-latest
    container:
      image: biorouter:local
      env:
        BIOROUTER_PROVIDER: openai
        BIOROUTER_MODEL: gpt-5.6
        OPENAI_API_KEY: ${{ secrets.OPENAI_API_KEY }}
    steps:
      - uses: actions/checkout@v4
      - name: Run BioRouter analysis
        run: |
          biorouter run -t "Run the QC checks on the sequencing results in this repo and summarize any flagged samples"
```

### GitLab CI

```yaml
analyze:
  image: biorouter:local
  variables:
    BIOROUTER_PROVIDER: openai
    BIOROUTER_MODEL: gpt-5.6
  script:
    - biorouter run -t "Generate a methods writeup for the analysis pipeline in this project"
```

## Image Details

### Size and Optimization

- **Multi-stage build**: two builder stages are discarded (`golang:1.26.8-bookworm`, which builds the native Biorouter Copilot helper, and `rust:1.92-bookworm`, which builds the CLI); only the compiled artifacts are copied forward
- **Base image**: `debian:bookworm-slim`, pinned by digest, plus eleven runtime packages (`ca-certificates`, `libssl3`, `libdbus-1-3`, `libxcb1`, `python3`, `python3-gi`, `gir1.2-atspi-2.0`, `gir1.2-gtk-3.0`, `at-spi2-core`, `curl`, `git`). The five in the middle are the Biorouter Copilot helper's AT-SPI runtime
- **Optimizations**: the `Dockerfile` overrides the release profile with LTO, one codegen unit, `opt-level=z`, and stripping
- **Contents**: the CLI (`/usr/local/bin/biorouter`) and the Biorouter Copilot helper (`/usr/libexec/biorouter/computer-use`) — it builds `--package biorouter-cli`, so the `biorouterd` daemon is **not** in the image

Image and binary sizes are not stated here on purpose: they move with the profile settings and the dependency tree, and a stale number is worse than none. Measure your own build with `docker images biorouter:local`.

### Security

- Runs as non-root user `biorouter` (UID 1000)
- Minimal attack surface with only essential runtime dependencies
- **No automated rebuilds.** Nothing builds or publishes this image on a schedule, so its base-image CVE exposure is whatever `debian:bookworm-slim` carried at the digest pinned in the `Dockerfile`. Rebuild the image yourself to pick up base-image updates.

### Included Tools

The image includes essential tools for BioRouter operation:
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
