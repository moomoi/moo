# moo.moi

`web/` is the moo.moi site: a Tish program compiled to a native HTTP server, like the app with no
Node and no JavaScript. Its pages are HTML and CSS only, and a Content Security Policy forbids
script.

| Path | Response |
| --- | --- |
| `/` | Landing page |
| `/get` | Download page (the button goes to `/download`) |
| `/download` | `302` to the latest DMG on GitHub |
| `/docs`, `/docs/<page>` | User documentation (below) |
| `/marketplace` | Plugin marketplace (the `PLUGINS` list in `main.tish`) |
| `/marketplace/<id>` | A plugin's page; Install opens `moo://plugins/<id>` |
| `/legal`, `/legal/terms`, `/legal/privacy` | Legal pages |
| `/callback` | Sign-in relay (below) |
| `/health` | `200 ok`, for load balancers |
| anything else | 404 page; methods other than GET and HEAD get 405 |

## The sign-in relay

Hypery only redirects to registered URLs, and the Moo app (`app_1791139621564_l2l6kzx0m`) has
exactly one: `https://moo.moi/callback`. Moo signs in like this:

1. Moo listens on `127.0.0.1:<free port>` and opens Hypery's authorize page with
   `redirect_uri=https://moo.moi/callback`, a PKCE challenge, and `state=<port>.<nonce>`.
2. Hypery sends the browser to `https://moo.moi/callback?code=…&state=…`.
3. moo.moi checks that `state` is a port from 1024 to 65535, a dot, and a 16–128 character
   base64url nonce, then answers `302` to `http://127.0.0.1:<port>/callback?<the same query>`
   with `Cache-Control: no-store` and `Referrer-Policy: no-referrer`. Anything else gets a 400
   page. Error callbacks (`error=access_denied`) are relayed the same way so Moo can report them.
4. Moo checks the whole `state` and exchanges the code, sending the same `redirect_uri` and its
   PKCE verifier.

The server keeps no state and logs nothing about requests. A code passing through it can't be
redeemed without the verifier, which never leaves the Mac.

Moo picks the redirect from the provider's `redirectUri` in `shortcuts.json`, then
`MOO_HYPERY_REDIRECT`, then the built-in `https://moo.moi/callback`. The client id comes from
`MOO_HYPERY_CLIENT_ID` or the built-in one.

## Docs

The user docs at `/docs` are written as markdown in `web/docs/<NN-section>/<NN-page>.md`. Each
section folder has a `_section.txt` holding its name, and each page starts with `title` and
`summary` frontmatter. Number prefixes set the order and are dropped from URLs, so
`web/docs/02-using/03-clipboard.md` is `/docs/clipboard`.

The server has no filesystem access and its pages carry no script, so `web/gen-docs.tish` renders
the markdown at build time into `web/src/docs.tish` (generated; don't edit). `build.sh` runs it
first, or run it alone with `tish run --feature fs gen-docs.tish` from `web/`. The header of
`gen-docs.tish` lists the markdown it understands, including `> **Tip**` callouts and `[[⌘ K]]`
keycaps. Code blocks are highlighted for `tish`, `json` and `sh`.

## Build and run

```sh
bash web/build.sh                  # web/dist/moo-web
PORT=8080 web/dist/moo-web         # default port 8080
```

To try a sign-in against a local copy, point Moo at it:
`MOO_HYPERY_REDIRECT=http://127.0.0.1:8080/callback`. That URL must also be registered with the
OAuth app, so for real Hypery this only works with `https://moo.moi/callback`.

`cargo test --lib` in `packages/moo-macos` covers the relay flow with a stand-in relay. To run the
real server through Moo's sign-in code:

```sh
MOO_TEST_RELAY=http://127.0.0.1:8080/callback cargo test --lib -- --ignored moo_web_relay
```

## Deploy

moo.moi is the Vercel project `moo-web` (team Knoeone), running the server as a container
function. To ship a change:

```sh
cd web && vercel deploy --prod
```

Vercel builds `web/Dockerfile.vercel`. It installs the released compiler from npm
(`@tishlang/tish`, the same version as `package.json`) in a Node stage, compiles `src/main.tish` and runs
the binary on distroless, listening on port 80. `web/vercel.json` declares the Dockerfile as a service and sends every path to it.
Without that file Vercel deploys `web/` as static files. A build takes about 5 minutes.

Preview deployments (`vercel deploy` without `--prod`) sit behind Vercel's login; test them with
`vercel curl /health --deployment <url>`.

The same Dockerfile runs anywhere else: `docker build -f Dockerfile.vercel -t moo-web web`. The
relay page has to be served over HTTPS at exactly `https://moo.moi/callback`.
