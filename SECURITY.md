# Security policy

evalsig can act as a referee for AI agents, so a way to bypass the referee
(reading sealed labels, replaying a gate, restoring a spent budget, forging a
ledger line) is a security issue, as is any way for untrusted input files to
crash the importer or exhaust memory.

## Reporting a vulnerability

Report vulnerabilities privately through GitHub: open the repository's
**Security** tab and choose **Report a vulnerability**. Do not open a public
issue, pull request or discussion for a suspected vulnerability.

Include the evalsig version (`evalsig --version`), the platform, and the
smallest set of files and commands that reproduces the problem. Use synthetic
data; do not send real evaluation records.

You can expect an acknowledgement within seven days. Fixes are released as a
patch version, and the advisory is published once a fixed release is
available.

## Supported versions

Before 1.0, only the latest release receives security fixes.
