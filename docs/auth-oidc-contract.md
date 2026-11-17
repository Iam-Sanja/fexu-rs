# OIDC provider and account-linking contract

[Overview](index.md) · Current providers: [auth-providers.md](auth-providers.md) · Threat model: [auth-threat-model.md](auth-threat-model.md)

This document is the binding design for a future OpenID Connect authorization-code login. It is not an implementation. `RemoteProtocol::Oidc`, `RemoteProtocol::OAuthAuthorizationCode` and `RemoteProtocol::Passkey` stay `is_implemented() == false`. No route accepts an ID token, an authorization code, or a provider subject. No type in `fexu-auth` performs remote login.

The contract describes the local password, MFA, session and cookie behavior already on `main`. That includes `provision_local`, `suspend_local`, `restore_local`, `revoke_local_sessions`, transactional deactivation, single-use reauthentication proofs, and application cookie namespaces. A later implementation must meet the rules below. This document does not change `IdentityProvider`, cookies, CSRF, admin routes, migrations, the OpenAPI document, or SQLx. Future HTTP failures stay on the existing `problem_json` contract. This design adds no security scheme and no route.

## Already implemented

These controls exist for local password accounts and stay the only login path:

| Boundary | Current code |
|---|---|
| Provider verification | `IdentityProvider::verify` checks an email and a secret. `PasswordProvider` is the only implementation. Unknown emails pay the dummy Argon2 hash. Inactive users return `InvalidCredentials`. |
| Identity result | `VerifiedIdentity` carries `user_id`, `email` and `method`. It has no `is_staff`, `is_superuser`, or role field. |
| Session issuance | `login` and `login_replacing` create a new session only after `PasswordProvider` verification. Confirmed TOTP returns `MfaPending`, sets no session cookie, and does not revoke the anonymous session. When a session is issued, the anonymous session is revoked in that transaction and is never promoted. The cookie name comes from `AuthConfig`, including `with_cookie_namespace`. |
| Local account | `create_user`, `LocalAccountLifecycle`, `set_user_active` and `close_account` own the user row. `set_user_active(false)` and `suspend_local` deactivate the user, revoke sessions and delete pending login challenges in one transaction. `revoke_local_sessions` revokes sessions and deletes those challenges while the account stays active. Close keeps the row so the email stays reserved and does not delete files. |
| Local authorization | `authorize` reads `is_active`, the stored superuser flag, and role permission codes. `is_staff` does not grant model permissions. Staff and superuser are columns on the local user, written by the admin or CLI, not by a provider result. |

`PasswordProvider` does not insert a session and does not create a user. A local contract test locks that boundary.

## Not implemented

OIDC, OAuth authorization-code, and passkeys are specified here only as future work. A later change must not report success, must not add a provider that returns `VerifiedIdentity` from an unverified token, and must not flip `RemoteProtocol::is_implemented` until the mock tests in this document pass.

`IdentityProvider::verify(email, secret)` is the password shape. OIDC must not be added as another implementation of that method. An authorization code is not a password, and an email claim is not the account key. The future protocol gets its own types, listed below, in a later pull request.

## Five boundaries

A future login crosses five separate steps. Collapsing any two of them is an account takeover or a privilege escalation.

1. **External identity.** The pair `(issuer, subject)`. `issuer` is the configured issuer URL. `subject` is the `sub` claim. The pair is stable for that provider. It is not an email address, not a display name, and not a local user id.
2. **Local account.** The existing `_fexu_users` row: id, normalized email, password hash, `is_active`, `is_staff`, `is_superuser`. Only local administration changes staff, superuser, and roles.
3. **Account link.** A row that maps one external identity to one local user id. The link is created only by the explicit linking rules below. It is not inferred.
4. **Session issuance.** The existing session insert: new session id, new token, privileges copied from the local user row at authentication time. The provider does not receive a session cookie and does not choose the session id. The cookie name is the configured name for that application.
5. **Local roles and object permissions.** `assign_role`, `grant_permission`, and object policies. Provider claims named `groups`, `roles`, `admin`, or `staff` are ignored.

An external provider must not set `is_staff` or `is_superuser`, must not call `assign_role` or `grant_permission`, and must not write those columns. A compromised or malicious identity provider then yields an ordinary local user, or no user, never an administrator.

## Proposed types

These shapes are normative for the implementation issue. They are intentionally absent from the crate so a stub cannot be called.

```text
ExternalIdentity { issuer, subject }
LocalAccountId  = existing user id
AccountLink     { local_user_id, issuer, subject }
LinkDecision    = Linked(LocalAccountId)
                | Unlinked
                | Refused
SessionIssuer   = existing issue_login_session, after Linked and is_active
```

`ExternalIdentity` does not contain privileges. `AccountLink` does not contain an email. `LinkDecision::Linked` names a local user and nothing else. Session issuance stays the existing function and still loads staff, superuser, and roles from the database.

Lookup key for a link: exactly `(issuer, subject)`. Email is not an index that selects a user during login.

## Authorization-code flow with PKCE

The resource server is Fexu. The browser is the user agent. The identity provider is configured per deployment, not taken from the request. Production accepts only `https` issuers. A loopback issuer is allowed only in the test configuration and must be rejected when `Environment::Production` is set.

No log line, tracing field, or error string may contain an authorization code, refresh token, access token, ID token, client secret, PKCE verifier, session token, nonce, or email address. Failures log a stable error class and the configured provider name.

### Start

1. The browser sends a state-changing request to start login. That request uses the existing CSRF check and the configured cookie names, including a namespace from `with_cookie_namespace`. This contract does not change cookie or CSRF code and does not hard-code `fexu_session`.
2. Fexu draws `state` and `nonce` from the system random source, at least 128 bits each, and stores only their hashes, bound to the current anonymous session id, with a 10-minute expiry. Raw values are placed only in the redirect that leaves the server.
3. Fexu draws a PKCE `code_verifier` of 43 to 128 unreserved characters and stores it server-side on that same record. The authorize request carries `code_challenge = BASE64URL(SHA256(verifier))` and `code_challenge_method=S256`. The plain method is rejected.
4. `redirect_uri` is the one configured value. A query parameter cannot replace it. Comparison is an exact string match. Wildcards, open redirects, and user-supplied URIs are rejected.
5. The authorize URL contains the configured `client_id`, `redirect_uri`, `response_type=code`, `scope` including `openid`, `state`, `nonce`, and the S256 challenge. Fexu does not put the verifier or the client secret in that URL.

If storage of the transaction fails, Fexu redirects nowhere and creates no session.

### Callback

1. The identity provider redirects the browser to the configured redirect URI with `code` and `state`, or with an error. An error, a missing code, or a missing state stops the flow. No session is issued.
2. Fexu loads the stored transaction by the state hash. Unknown, expired, or already consumed state is a failure. The anonymous session id on the callback must be the one stored at the start. A copied callback in another browser fails.
3. State is consumed before the token request, in a conditional update that succeeds for one caller. A second concurrent callback sees the consumed row and fails.
4. Fexu sends the code, the configured redirect URI, and the stored verifier to the token endpoint. The client secret, when the provider is confidential, stays in server configuration and is not logged. A PKCE mismatch, including a verifier that does not match the stored challenge, is a failure.
5. The token response is not trusted as a session. Fexu validates the ID token and then discards the access token and the refresh token. They are not written into the session cookie or the session row.

### ID token

Validation order is fixed. A later check must not run if an earlier check fails. Email is not read until the signature and the identity claims have passed, and even then it is not the lookup key.

1. Reject `alg=none`. Reject any algorithm outside the configured allowlist. The default allowlist is asymmetric algorithms published in the provider metadata and present in the trusted JWKS. Symmetric algorithms are rejected for this public token. An embedded `jwk` in the token header is rejected.
2. Select the key by `kid` from the trusted JWKS document fetched from the configured `jwks_uri` over HTTPS. The URI itself comes from the pinned discovery document or from configuration, not from the token.
3. Verify the signature with that key. Unknown `kid` triggers one JWKS refresh. If the key is still unknown, reject the token. A cached document has a bounded lifetime. A fetch failure, a timeout, or a non-HTTPS URI fails the login. A failed fetch is not cached as an empty key set.
4. `iss` must equal the configured issuer. A different issuer, including one that shares a subject or an email, is a different provider and is rejected.
5. `aud` must contain the configured client id. When several audiences are present, `azp` must equal that client id. A token for another client is rejected.
6. `exp` is required and must be in the future, allowing at most 60 seconds of clock skew. `iat` is required and must not be more than 60 seconds in the future. `nbf`, when present, is enforced with the same skew. Larger skew is a failure, not a configuration the request can choose.
7. `nonce` must equal the stored nonce. A missing nonce is a failure.
8. `sub` must be present and non-empty. Only then does Fexu form `ExternalIdentity`.
9. When `jti` is present, a repeat of that `jti` until `exp` is a failure. The authorization code remains single-use even when `jti` is absent.

### After a valid token

1. Resolve the link by `(issuer, subject)` using the rules in the next section.
2. On `Unlinked` or `Refused`, issue no session and do not write staff, superuser, or roles.
3. On `Linked`, load the local user. If the user is missing or `is_active` is false, issue no session. Do not reactivate the user.
4. If the local user has confirmed TOTP, stop with the existing `MfaPending` challenge. Do not set a session cookie and do not revoke the anonymous session. The OIDC result does not skip the second factor. A later `complete_pending_login` still refuses an inactive user and consumes the challenge.
5. Otherwise call the existing session insert only when `is_active` is true in that same transaction. Reading the flag earlier and inserting later is not enough, because `suspend_local` can commit in between. The new session id must differ from the anonymous session id. The anonymous session is revoked only in that insert, which is the same fixation rule as password login. The issued cookie uses the configured session name.
6. Privileges on the issued session come from the local row and from role tables. They are not taken from the token.

### Logout, replay, and provider failure

Local logout stays `logout` of the presented session token. It revokes that Fexu session. It does not require the identity provider to be reachable. A repeated callback finds the state or the code already consumed and does not create a replacement session.

Provider-initiated logout, including back-channel logout, is not part of the first implementation. Until that exists, a logout at the identity provider must not be documented as a Fexu session revoke.

Discovery failure, JWKS failure, token-endpoint failure, timeout, and malformed JSON all fail closed. They do not fall through to email matching, do not create a link, and do not create a user.

Parallel password-style sessions remain allowed: two successful logins for the same linked user create two Fexu sessions. Two callbacks that present the same code or the same state do not. One consume wins; the other fails.

## Account linking

Email equality never selects an existing local user and never writes a link. The same address at a provider and in `_fexu_users` is a coincidence until the local user proves control of the local account.

| Situation | Required result |
|---|---|
| First login, no link | `Unlinked`. Default policy creates neither a user nor a link. A deployment may later opt in to creating an inactive, non-staff, non-superuser user with no roles, but that switch is off and is not part of this contract's implementation. Silent creation of an active user is forbidden. |
| Local account already uses that email | `Refused`. The user may link only after a local password login, and TOTP when it is enabled, on a live session, followed by an explicit confirmation. That confirmation consumes the existing reauthentication proof. The proof is bound to that user and that session, lasts at most 300 seconds, and is accepted once in the current process. The confirmation stores `(issuer, subject)` for that session's user id. |
| Identity already linked | `Linked` to that local user. Claims do not change email, staff, superuser, or roles. |
| Second provider | A new `(issuer, subject)` is a new link. It does not replace another issuer. The same `sub` at two issuers is two identities. Linking the second provider requires the same local reauthentication and confirmation. |
| Provider changes the email claim | The link stays on `(issuer, subject)`. The local email does not change. A local email change stays a separate, already-authenticated account operation. |
| Local account disabled or closed | `Refused`. No session. Close already leaves the user row in place so the address stays reserved. A later provider login must not create a second user for that address and must not set `is_active`. |
| Remove one link | The caller has a live session for that local user and passes the existing reauthentication check. The operation deletes that one link. It does not delete the user, sessions, roles, or other links. Unlink is refused when the user would have no remaining local password and no other link. Password accounts on `main` have a password hash, so a single OIDC unlink must leave that password in place. |
| Close the account | The existing close transaction stays authoritative. The future link table is deleted in that same transaction. Sessions are revoked. The row stays. Files are still not deleted here. |

Provider claims are not authorization input. There is no mapping from a claim value to `is_staff`, `is_superuser`, a role name, or a permission code.

## Test matrix

Every row below must be executable against a local mock identity provider. The mock signs tokens with a test key, serves JWKS on loopback or in process, and uses no external account and no production client secret. Browser coverage is additional and does not replace these checks.

| Threat | Required failure or result | Local mock |
|---|---|---|
| Forged ID token | Signature check fails. No session, no link. | Yes |
| Wrong issuer | Reject before link lookup. | Yes |
| Wrong audience or client id | Reject before link lookup. | Yes |
| `alg=none` or an algorithm outside the allowlist | Reject. | Yes |
| Embedded `jwk` | Reject. | Yes |
| Tampered or unknown `state` | Reject. Stored transaction stays unconsumed only when the state does not match; a match consumes it once. | Yes |
| Missing or different `nonce` | Reject. | Yes |
| PKCE verifier mismatch or plain method | Reject. No token is accepted. | Yes |
| Expired `exp`, future `iat` beyond 60 seconds, or `nbf` in the future | Reject. | Yes |
| JWKS rotation | Current `kid` succeeds. Retired `kid` fails. Unknown `kid` refreshes once and then fails. | Yes |
| Provider confusion | Same `sub` and same email at issuer B do not select a link for issuer A. | Yes |
| Email account takeover | Verified email equal to a local user does not link and does not issue a session. | Yes |
| Privilege claim | A claim naming staff, superuser, or a role leaves the local flags and role table unchanged. | Yes |
| Concurrent callback | One caller consumes the code and state. The other fails. At most one new session. | Yes |
| Session fixation | Issued user session id differs from the anonymous session id. A pre-existing user session cookie is not adopted. | Yes |
| CSRF on the start request | Existing CSRF rejection. No transaction is stored. | Yes, through the current CSRF check once a route exists |
| Logout then replay | Local logout revokes the issued session. Replaying the callback does not issue another. | Yes |
| Disabled or closed local user | Valid token and a real link still issue no session and do not reactivate the user. | Yes |
| Provider outage | Discovery, JWKS, or token-endpoint failure issues no session and writes no link. | Yes |
| Log redaction | Captured logs contain none of the token, code, verifier, secret, nonce, or email. | Yes |

The mock must be able to mint a correctly signed token, a token with one field altered, an expired token, and a second key for rotation. Tests supply those tokens directly to the validator once it exists. They do not call a public identity provider.

## Follow-up work

Implementation is split so none of it can be described as done by this document:

1. #172 — authorization code, PKCE, state, nonce, token validation, JWKS, and session issuance for an already linked active user.
2. #173 — link storage and the linking rules, including the email-takeover refusal.
3. #174 — the local mock and, separately, browser tests of the redirect and the existing CSRF check.
4. #175 — optional cross-process rate limiting and `jti` replay storage. Single-use state and single-use authorization codes are required in the database before this item. Process-local login limits remain the current residual until that item lands.

Passkeys stay unimplemented and are not part of these four items. Back-channel logout is not part of item 1.
