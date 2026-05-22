# sparrow-chef

Bootstrap tool for coding agents. Runs once to scaffold a complete SparrowDB project, pull the Docker image, start the database, and seed it with example data — so an AI agent can start building immediately without manual setup steps.

## Install

```bash
cargo install sparrow-chef
```

## Build from source

```bash
cargo build -p sparrow-chef --release
# binary at: target/release/sparrow-chef
```

## Usage

```bash
# Interactive mode — prompts for project path and build intent
sparrow-chef chef

# Automatic mode — uses defaults, no prompts (alias: cook)
sparrow-chef chef --auto
sparrow-chef cook --auto
```

## What it creates

Running `sparrow-chef chef` writes the following into the chosen project directory:

```
my-project/
  docker-compose.yml          # SparrowDB service definition
  db/
    schema.hx                 # starter node/edge schema (User, Follows)
    queries.hx                # starter read/write queries
  examples/
    seed.json                 # example write request payload
    read.json                 # example read request payload
  SPARROWDB_CHEF_PROMPT.md    # prompt context file for the coding agent
```

After writing files, it runs `docker compose up -d` and waits for the database to become healthy, then sends the seed request.

## Requirements

- Docker (for starting the database)
- Internet access (to pull `ghcr.io/sparrowdb/sparrowdb:latest`)
