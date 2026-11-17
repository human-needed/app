# Security policy

## Supported versions

Only the latest release gets security fixes. The app updates itself, so make sure
**Settings → App → Update the app automatically** is on.

## Reporting a vulnerability

Please **don't open a public issue**. Report it privately on GitHub:
[Security → Report a vulnerability](https://github.com/human-needed/app/security/advisories/new).

Include what you found, how to reproduce it, and what an attacker could do with it. We'll reply
within a few days, keep you updated, and credit you in the release notes if you'd like.

## What's in scope

- The app's local hub on `127.0.0.1:24816` (requests from browsers and other origins are refused).
- The cloud link to the HumanNeeded relay, and the secret it keeps in the OS credential store.
- App updates (they're signed; the app refuses anything else) and firmware updates for the puck.
- The installers and the release workflow.
