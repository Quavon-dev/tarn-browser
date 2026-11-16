# Maintainers

Tarn is maintained by Quavon UG (haftungsbeschränkt). This file lists the people who can reach the
project's sensitive resources and what each role is responsible for. How access is granted and
reviewed is in [`CONTRIBUTING.md`](CONTRIBUTING.md), "Maintainers and access"; the secrets
themselves are described in [`docs/release.md`](docs/release.md), "Secrets and credentials".

## People

| GitHub | Role | Access |
|---|---|---|
| [@rgxdev](https://github.com/rgxdev) | Maintainer, organisation owner | Admin on the repository; member of `@Quavon-dev/maintainers`, so a Code Owner and an approver of `release-signing` |
| [@MadebyLeopold](https://github.com/MadebyLeopold) | Maintainer, organisation owner | Same as above |
| [@quavondev](https://github.com/quavondev) | Maintainer, organisation owner | Same as above |

Organisation owners can change repository and organisation settings, rulesets, environments and
their secrets. GitHub never shows a secret's value, but an owner who changes an environment's rules
or workflows could make a job reveal it, so owners are trusted with every secret. The GitHub Apps
with access are listed in `docs/release.md`, "Repository settings".

## Roles

**Contributor**: anyone who opens an issue or pull request. Follows
[`CONTRIBUTING.md`](CONTRIBUTING.md) and, when an agent helps, [`AGENTS.md`](AGENTS.md); answers
for their contribution and its licence terms.

**Maintainer** (write access or above, member of `@Quavon-dev/maintainers`):
- votes on issues (`CONTRIBUTING.md`, "Voting") and reviews pull requests; a pull request needs
  the approval of a maintainer who did not write it, and Code Owner review for protected paths;
- keeps `main` releasable: CI green, advisories and code scanning alerts handled within the
  thresholds in [`SECURITY.md`](SECURITY.md);
- triages security reports sent through GitHub private vulnerability reporting, coordinates the fix
  and publishes the advisory (`SECURITY.md`, "Coordinated disclosure");
- approves `release-signing` deployments only for a release they mean to ship
  (`docs/release.md`);
- rotates secrets as `docs/release.md` describes and reviews this list once a year.

**Tarn maintainer** (write access through `@Quavon-dev/tarn-maintainer`; no members yet):
- pushes branches and opens pull requests;
- reviews pull requests, and their approval counts as the non-author approval `main` requires
  (not as Code Owner review);
- labels, assigns, closes and reopens issues and pull requests, and votes on issues (the Issues
  workflow counts every account with write access, `CONTRIBUTING.md`, "Voting");
- cannot merge into `main` (only `@Quavon-dev/maintainers` may update it, ruleset "main
  merges"), create `v*` tags, approve a `release-signing` deployment, read secrets or change
  settings. GitHub's write role on this plan also lets them start workflows and edit GitHub
  releases; signing and publishing still need a maintainer's approval.

**Organisation owner**: a maintainer who also manages the organisation, its members, two-factor
requirement, rulesets, environments and secrets, and changes them only as `docs/release.md`,
"Repository settings", describes.
