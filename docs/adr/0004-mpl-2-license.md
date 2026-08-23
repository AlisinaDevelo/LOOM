# ADR 0004: Mozilla Public License 2.0

- Status: Accepted
- Date: 2026-08-23

## Context

LOOM is intended to be a public repository with a reusable local core. The project needs a clear
license for source files, modifications, and larger works while keeping modifications to covered
files available under the same license.

## Decision

Distribute LOOM under the Mozilla Public License 2.0. Keep the complete license text in the root
LICENSE file and preserve the MPL source-file notice where it is added to covered source files.
The package metadata and citation file identify MPL-2.0.

## Consequences

- Recipients have a clear grant for covered source, with the obligations described by the MPL.
- Modifications to covered files remain under the MPL when distributed.
- Larger works can have their own terms while complying with the MPL for covered software.
- Contributors and downstream distributors must review the license text and preserve notices.

This record is a repository decision, not legal advice. See the
[official Mozilla MPL 2.0 text](https://www.mozilla.org/en-US/MPL/2.0/).
