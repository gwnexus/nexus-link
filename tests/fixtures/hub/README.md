# Hub contract fixtures

Golden files published by the Nexus Hub (`gwnexus/nexus-hub`) and vendored
here unchanged, so a contract change on either side fails CI here.

| File | Hub path | Hub commit | Git blob |
|---|---|---|---|
| `nexus-cmd-v1.vectors.json` | `contracts/node-commands/nexus-cmd-v1.vectors.json` | `d5228b4bfeca26c975662b78a68b72c064cb2a6c` | `cf0fc6c896df21565dc4b97024654e85dfd41237` |

Used by `tests/src/hub_vectors_tests.rs` (ADR-0007, Dispatch c2696a25). The
pinned commit is repeated there as `HUB_COMMIT`.

## Updating

Never edit the JSON by hand. Fetch it at the new hub commit, check that the
git blob matches the hub, then update the table and `HUB_COMMIT`:

```sh
REF=<hub commit>
gh api "repos/gwnexus/nexus-hub/contents/contracts/node-commands/nexus-cmd-v1.vectors.json?ref=$REF" \
  -q .content | base64 -d > tests/fixtures/hub/nexus-cmd-v1.vectors.json
git hash-object tests/fixtures/hub/nexus-cmd-v1.vectors.json
gh api "repos/gwnexus/nexus-hub/contents/contracts/node-commands/nexus-cmd-v1.vectors.json?ref=$REF" -q .sha
```
