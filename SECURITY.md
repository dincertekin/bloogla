# Security policy

## Supported versions

Security fixes go into the latest release. Update from **Settings → Updates**
(or see [Updating](docs/running.md#updating)) to get them.

## Reporting a vulnerability

Please **don't open a public issue** for security problems. Report them
privately on GitHub instead: go to the
[Security tab](https://github.com/dincertekin/bloogla/security) and click
**Report a vulnerability**.

Helpful to include:

- what an attacker could do, and what they need first (an account? a role?);
- steps to reproduce, or a small proof of concept;
- the Bloogla version (Settings → Updates) and how it runs (binary or Docker).

You'll get a reply as soon as possible. Once a fix is released, you're
credited in the release notes unless you'd rather not be.

## How Bloogla protects sites

[docs/security.md](docs/security.md) describes the built-in protections:
the Content Security Policy, CSRF checks, rate limits, two-factor login,
encrypted secrets and signed updates.
