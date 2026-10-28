# Security Policy

BlueLink handles private notes, device identity keys, peer-provided paths, and encrypted synchronization. Please report security problems privately so users have time to update before technical details become public.

## Supported versions

BlueLink is pre-alpha. Security fixes currently target the latest commit on `main`; there are no supported release branches yet.

## Report a vulnerability

The preferred channel is GitHub's private vulnerability reporting form. If the form opens for you, use:

**[Report a vulnerability privately](https://github.com/semeikhan-t/bluelink/security/advisories/new)**

Repository maintainers should enable **Settings → Security → Private vulnerability reporting** so this route is available.

If the private form is unavailable, open a minimal public issue titled `Security contact requested` without exploit steps, keys, private notes, logs, or affected filenames. A maintainer can then arrange a private channel.

Include, when safe:

- the affected commit or version;
- the component and platform;
- impact and prerequisites;
- minimal reproduction steps or a proof of concept;
- whether device keys, vault contents, paths, or trust state may be exposed;
- any suggested mitigation.

Do not attach a real `.bluelink/device.json`, private vault, device key, or unredacted sync log.

## What to expect

Maintainers will acknowledge the report, reproduce and assess it, coordinate a fix, and agree on disclosure timing when possible. Response times are best-effort while the project is pre-alpha.
