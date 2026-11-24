# Middleware

**Current development:** 0.3.0-dev.1. Published: 0.2.0.

## Responsibility

`fexu-middleware` applies Tower layers to an Axum router. It does not authenticate users and it does not authorize a request. Cookie-write CSRF is installed by `App::into_router` in `fexu-api`, before `with_state`, so generated projects and `allow_anonymous` routes receive it without a handler calling `guard_cookie_write`. This crate does not create an anonymous session and does not reject a cookie-less POST.

## Crates and files

- `crates/fexu-middleware/src/lib.rs`
- Application: `fexu::web::prepare` and `examples/myshop/src/main.rs`

## Public API

`Standard` carries `body_limit` (standard 2 MiB) and `referrer_policy`. `apply(router, standard)` attaches the layers. `RequestId` is in the request extensions. `logged_path` remains a raw path-only helper. Standard access logging records the Axum matched route template, or `<unmatched>` when unavailable, rather than the raw path. A single incoming `x-request-id` is kept only when it is 8 to 64 ASCII letters, digits or hyphens. Duplicate or invalid values are replaced. A valid caller-supplied ID remains untrusted data.

## Data flow

Tower runs the last layer added first. `apply` is therefore attached in this order: security header, access log, request ID, body limit. From the outside inwards:

1. Body-Limit
2. Request-ID
3. Access-Log
4. Security headers
5. Cookie-write CSRF inside `App::into_router`, then the route and its authorization check

## Configuration

`Standard { body_limit, referrer_policy }` is the explicit configuration. `fexu::web::prepare` fills a missing referrer policy with `no-referrer` and does not read a TOML switch. Callers that use `apply` directly can select one of `no-referrer`, `same-origin`, `origin-when-cross-origin`, `strict-origin` or `strict-origin-when-cross-origin`. There is no switch that turns off the security headers or that trusts `Host` or `X-Forwarded-*`.

## Security

The standard access event contains method, route template, status, duration and request ID. It does not read query values, dynamic path parameters, cookies, `Authorization`, CSRF or request bodies. Application-defined tracing events remain the application’s responsibility; caller-supplied request IDs must not contain secrets. This layer owns the final `X-Content-Type-Options: nosniff` and `X-Frame-Options: DENY`. A handler cannot weaken either value. `Referrer-Policy` is also owned here. Every header field and every comma-separated token is read. The response keeps one canonical header only when every token is the same listed policy. A missing header, an unsafe token, or two different values are replaced by the configured policy. Repeated copies of one safe policy collapse to that single header. This keeps `same-origin` on the admin login document and `no-referrer` on the password-reset pages. A content security policy is not set so that the admin does not break. Origin checks, Fetch Metadata and CSRF live in `fexu-auth` and `fexu-api`, not in this crate. Neither this crate nor the cookie-CSRF layer replaces an authorization check. A cookie-less `allow_anonymous` write remains public unless the app calls `require_browser_writes`.

The `x-fexu-trace-order` header is a test probe and does not affect routing.

## Error cases

A body above the limit is 413 before the route runs. A request ID that could not be represented as a header value is not set. UUIDs are displayable.

## Example

```rust
let router = fexu::middleware::apply(router, fexu::middleware::Standard::default());
```

## Tests

`cargo test -p fexu-middleware` checks the request ID, the security headers after the whole `apply` chain, preservation of `same-origin`, preservation of recovery `no-referrer` when the default is `same-origin`, replacement of `unsafe-url` and the other unsafe policies, the forced `nosniff` and `DENY` values, the order `request_id,access_log,security_headers`, the body limit and that query and cookie are not in the response stream of the log path. The admin login walk over a real socket is `sqlite_browser_security_matrix` in `fexu-api`.

## Limitations

Standard Axum and Tower layers can supplement the built-in middleware. There is no declarative middleware list in `fexu.toml`. Logging uses `tracing` without an additional text-filtering pass.

## Extension

Additional layers can run before or after `apply`. Outer layers receive the request before the body limit is checked.
