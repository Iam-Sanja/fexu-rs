# Background jobs

Fexu includes persistent background job infrastructure with a worker runner and job inspection CLI.

Generate a worker:

```sh
fexu generate worker
```

Run the worker process separately from the web server:

```sh
fexu worker --concurrency 2 --poll-ms 1000
```

Inspect and operate jobs:

```sh
fexu jobs list
fexu jobs failed
fexu jobs inspect JOB_ID
fexu jobs retry JOB_ID --yes
fexu jobs cancel JOB_ID --yes
```

`fexu worker` runs the built-in supported worker behavior. To process application-specific job handlers, generate and configure a dedicated worker process with those handler registrations. A job's payload may contain sensitive business data, so `jobs inspect --show-payload` should be used only when authorized. See [CLI reference](../reference/cli.md).
