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
Usage: generative-model-proxy-server [OPTIONS]

Options:
  -b, --bind-to <BIND_TO>                                [default: 127.0.0.1:8080]
  -d, --database-folder <DATABASE_FOLDER>                [default: database]
  -o, --opentelemetry-endpoint <OPENTELEMETRY_ENDPOINT>
  -h, --help                                             Print help
  -V, --version                                          Print version

```

#### Monitoring

The server supports sending logs to an [OpenTelemetry](https://opentelemetry.io) compatible collector. This feature is a work in progress, and the output format will likely change in future releases.