# Adding a debrid service

Debrid (multi-hoster) services are the fully automatic route of BDownloader:
one account unlocks most hosters, with no wait and no captcha. Adding one is a
self-contained change in `crates/bd-core`.

Only use the service's **official, documented API**. Link the API
documentation in a doc comment above your method.

## Steps

### 1. Declare the service (`src/accounts.rs`)

Add a `Service` to `SERVICES`:

```rust
Service {
    id: "exampledebrid",            // stable id, used in accounts.toml and the keyring
    name: "ExampleDebrid",          // shown to users
    kind: ServiceKind::Debrid,
    needs_username: false,          // true if the API needs a login besides the secret
    secret_label: "API key",        // "API key", "API token" or "Password"
    help: "exampledebrid.com/account/api",   // where users find the secret
},
```

The position in `SERVICES` is the order in which debrid services are tried.
Never rename an existing `id`: it would orphan users' saved accounts.

### 2. Unlock links (`src/hosters.rs`)

Add an arm to `HosterResolver::try_debrid`:

```rust
"exampledebrid" => self.exampledebrid(url, &secret).await,
```

and the method:

```rust
/// https://exampledebrid.com/api/docs
async fn exampledebrid(&self, url: &Url, key: &str) -> Result<Resolved, String> {
    let body = self
        .json(
            self.client
                .post("https://api.exampledebrid.com/v1/unlock")
                .bearer_auth(key)
                .form(&[("link", url.as_str())]),
        )
        .await?;
    let link = body["download"].as_str().ok_or_else(|| api_error(&body, "error"))?;
    Self::resolved(link, body["filename"].as_str(), "ExampleDebrid")
}
```

- Return `Err` with a short readable message when the service cannot unlock
  the link; the resolver logs it and tries the next route.
- If the service prepares files asynchronously (like TorBox), poll with a
  bounded number of attempts, update the slot status
  (`slot.set_status(Some("ExampleDebrid: preparing…"))`) and stop when
  `slot.is_cancelled()`.
- Cache login tokens in `self.tokens` when the API uses sessions (see
  `mega_debrid_token`).

### 3. Check accounts

Add an arm to `HosterResolver::check_account` that calls the account endpoint
and returns `AccountCheck { premium, summary }`. Use the `summary(user,
premium, until)` helper so all services read the same way.

### 4. Test and document

- Unit-test any JSON parsing you factor out.
- Test by hand with your own account in the Accounts screen. Never commit
  keys, tokens or logs that contain them.
- Add the service to the feature list in `README.md`.
