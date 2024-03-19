# Generative Model Proxy Server

A work in progress.

Compile with `cargo build --release` (binary will be located in `./target/release`).

## Running

The server can be started by running the compiled binary with no arguments. A message containing first-time setup instructions should appear.

```
WARN generative_model_proxy_server:
  It looks like you don't have any users added to your database.
  Please see http://127.0.0.1:8080/admin/help (login with a blank
  username and "setup-key" as the password) for more information.
```

After first-time setup has been completed, HTML documentation regarding available API endpoints will be available at the `/admin/help` endpoint.

### Configuration

By default, the server will bind to `127.0.0.1:8080`, and will save the database in a folder located at `./database`. However, this behavior can be customized via CLI argments.

You can run the binary with the `-h` or `--help` arguments for a full list of available CLI arguments.

```bash
> ./generative-model-proxy-server --help
A multi-user proxy server for major generative model APIs

Usage: generative-model-proxy-server [OPTIONS]

Options:
  -b, --bind-to <BIND_TO>
          The internet socket address that the HTTP server will be available on [default: 127.0.0.1:8080]
  -d, --database-folder <DATABASE_FOLDER>
          The location of the folder used to store the proxy's database [default: ./database]
  -o, --opentelemetry-endpoint <OPENTELEMETRY_ENDPOINT>
          The OpenTelemetry-compatible collector used for logging. Signals sent to the collector may contain sensitive
          information
  -h, --help
          Print help
  -V, --version
          Print version
```

#### Monitoring

The server supports sending logs to an [OpenTelemetry](https://opentelemetry.io) compatible collector.

## Roadmap

- [X] Adding documentation
- [ ] Adding unit tests for major internal components
        - [ ] Rate limiter
        - [ ] Database
        - [ ] ModelRequest / ModelResponse
        - [ ] Admin API
- [ ] Adding integration tests
        - [ ] Model API
- [ ] Adding import functionality from https://github.com/cosmicoptima/openai-cd2-proxy
- [ ] Model rate limiting based on backend responses (such as HTTP headers)
- [X] Logging
        - [ ] Metrics
- [ ] Support more APIs
        - [ ] Chapter2
        - [ ] Anthropic
        - [ ] together.ai
        - [ ] tabbyAPI
- [ ] Support ChatCompletion(-like) -> Completion(-like) API conversion
- [ ] Support additional API key metadata
- [ ] Support preprocessing
- [ ] Support model listing
