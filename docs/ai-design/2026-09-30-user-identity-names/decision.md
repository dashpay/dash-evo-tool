# User identity names

On 2026-09-30, the user requested complete removal of device-local names for User identities. DashPay Display name is sufficient; labels fall back to a DPNS username, then a shortened identity ID. Create, load, profile settings, and owned-name screens must not offer a User identity alias. Existing serialized records remain readable.

This decision supersedes the local-nickname policy in G6/G7 and the Settings naming field in the [2026-04-22 identity redesign](../2026-04-22-identity-dashpay-redesign/design-spec.md). That document records the earlier design.

Masternode/Evonode administrative names and private contact nicknames remain supported.
