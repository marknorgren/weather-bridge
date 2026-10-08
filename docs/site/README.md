# Static API documentation

The GitHub Pages site documents the REST API, MCP tools, and canonical LikeC4 architecture. The API itself stays on AWS; GitHub Pages serves static files only. Contributor and adaptation guides remain source documentation linked from the repository [README](../../README.md).

## Preview

From the repository root:

```sh
python3 scripts/build-docs.py
python3 -m http.server 8791 --directory target/docs-site
```

Open http://127.0.0.1:8791. The build uses Python's standard library, the checked-in OpenAPI document, and the bundled Scalar 1.72.1 asset. It needs no Node, Rust build, CDN, network fetch, or credentials. Internal links are relative, so the site works under a repository subpath or at a domain root.

The default API origin is the script's `DEFAULT_API`. To document another deployment, such as the CloudFront URL:

```sh
python3 scripts/build-docs.py --api-url https://weather.example.com
```

Set the matching docs base URL on the API server so `/developer` links to the fork's reference:

```sh
cargo run --locked -- serve --docs-url https://OWNER.github.io/REPOSITORY/
```

The build adds this server to its copy of the OpenAPI document; the runtime file is unchanged. Only allowlisted docs files and the Scalar license go into `target/docs-site`. Deployment state, Terraform files and credentials are never copied.

## Rendering regression tests

The Pages workflow runs `scripts/test-docs.cjs` before uploading or publishing.
It opens the built REST reference in Chromium and WebKit at desktop and narrow
widths, checks Scalar heading and table sizes and grid/section spacing, sends a
request to a deterministic JSON fixture, and checks that the response has usable
rendering space. It also checks that the MCP guide retains its authored styles.
No live NWS or API requests are needed.

Run locally (Node.js 22 or newer):

```sh
python3 scripts/build-docs.py
npm install --prefix /tmp/weather-docs-browser --no-save --ignore-scripts playwright@1.62.1
export NODE_PATH=/tmp/weather-docs-browser/node_modules
export PLAYWRIGHT_BROWSERS_PATH=/tmp/weather-docs-browsers
node /tmp/weather-docs-browser/node_modules/playwright/cli.js install chromium webkit
node scripts/test-docs.cjs
```

Keep authored typography, table, grid and section rules scoped to `.wrap`;
Scalar's client can mount outside `#app`, so excluding only `#app` is insufficient.

## Architecture model

The site publishes the canonical LikeC4 model in [`docs/architecture/`](../architecture/README.md) at `architecture/`. The Pages workflow installs LikeC4 1.59.4 with Node 22, builds the model as one self-contained HTML file with hash routing, and passes it to the docs build. The page loads nothing from other sites and works under a repository subpath. To include it locally:

```sh
likec4 build docs/architecture --output-single-file --use-hash-history -o target/likec4
python3 scripts/build-docs.py --architecture target/likec4
```

Without `--architecture`, the build writes a placeholder page that says how to add the model, so the preview above still needs no Node. `just check-docs` performs the LikeC4 build, static site build, browser checks, and expected-view checks together.

## Publish on GitHub Pages

1. Push this repository to the chosen GitHub repository with `main` as the deployment branch.
2. In **Settings → Pages → Build and deployment → Source**, choose **GitHub Actions**.
3. Run **API docs on GitHub Pages** from the Actions tab, or push a documentation change to `main`.
4. The deployment job prints the Pages URL, usually `https://OWNER.github.io/REPOSITORY/` for a project site.

The workflow builds on pull requests that touch the docs, but publishes only from `main`. It uploads only `target/docs-site`, uses pinned action commits, and grants Pages write/OIDC permission only to the deployment job. No AWS credentials or repository secret is required. If the repository owner set environment protection rules, a deployment may need their approval.

The workflow passes the `WEATHER_BRIDGE_API_URL` repository variable (**Settings → Secrets and variables → Actions → Variables**) as `--api-url` when it is set, and otherwise uses the script default, which is the author's demo at `https://bridge.wx.mrkd.co`. Forks should set it to their own deployment's URL, then re-run the workflow.

The REST reference sends browser requests straight to the API. The API allows this with `Access-Control-Allow-Origin: *` on `GET` and `HEAD` for `/v1/*` and `/openapi.json`, and answers `/v1/*` preflight. The API is public, read-only and uses no cookies, so no credentials are involved.

`/mcp` keeps its Origin allowlist and rejects other sites' browsers. Live MCP tool discovery therefore stays on the API's own MCP tool explorer at `/developer#mcp`. The Lambda does not serve the REST reference or the Scalar bundle.

See [GitHub's custom Pages workflow documentation](https://docs.github.com/en/pages/getting-started-with-github-pages/using-custom-workflows-with-github-pages) for repository requirements and [Scalar configuration](https://scalar.com/products/api-references/configuration) for renderer options. This workflow does not set a custom domain or change DNS.
