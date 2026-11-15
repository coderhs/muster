# Security policy

## Reporting a vulnerability

Please do not open a public issue for a security problem. Report it privately through
GitHub: on the repository's **Security** tab, choose **Report a vulnerability**. Include
what you found, how to reproduce it and which version or commit you tested.

You should get a reply within a week. Once a fix is released, the report can be made
public, with credit to you if you want it.

## Scope

Muster runs on your own Mac for you alone. The daemon listens on `127.0.0.1` only and
requires a token. What it protects against, and what it does not, is described in
[`docs/wiki/Security.md`](docs/wiki/Security.md). An earlier review and the fixes made
for it are in [`docs/REVIEW-daemon.md`](docs/REVIEW-daemon.md).

Note that an agent with unrestricted shell access runs as you and can do anything you
can. Muster's limits on cards apply to well-behaved agents and to agents with restricted
tools. They cannot stop an agent that has unrestricted shell access.
