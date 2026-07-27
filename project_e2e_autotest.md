# Stable Channels Autotest Memory

## LSP restart crash-loop: LSPS2 persisted state

- Discovered 2026-07-18 on the e2e regtest stack, branch `autotest`.
- Best label: upstream `ldk-node` / `lightning-liquidity` LSPS2 persistence bug, surfaced by `ldk-server`.
- Impact: operational LSP may run normally until restart, then `ldk-server` aborts because embedded `ldk-node` cannot reload persisted LSPS2 liquidity state. Existing channel/fund state appears intact; this is primarily an LSP availability risk.
- Signature: channel monitors and ChannelManager load successfully, then startup fails with only `Failed to build LDK Node: Failed to read from store.`
- Relevant state: `ldk_node_data.sqlite`, table `ldk_node_data`, `primary_namespace='lightning_liquidity_state'`, especially `secondary_namespace='lsps2_service'`.
- Likely trigger: LSPS2 JIT channel reaches `PaymentForwarded`, then a splice causes another `ChannelReady`; LSPS2 logs `Channel ready received when JIT Channel was in state: PaymentForwarded`, and persisted peer state later fails reload.
- E2E recovery was verified by backing up the DB, deleting only `lightning_liquidity_state`, and restarting. Same node id, channels, funds, peers, and SC daemon connectivity came back.

Prod mitigation:

1. Use a quiet maintenance window.
2. Stop `stable-channels-lsp`, then `ldk-server`.
3. Back up the prod `ldk_node_data.sqlite`.
4. Run:

   ```sql
   DELETE FROM ldk_node_data WHERE primary_namespace='lightning_liquidity_state';
   ```

5. Start `ldk-server`, confirm the expected node id and channel state, then start `stable-channels-lsp`.

What is lost: disposable LSPS2/JIT bookkeeping, including in-flight JIT open promises. Users mid-onboarding may need to retry. Channel monitors/ChannelManager state should not be touched.

Keep the bad e2e DB artifact until upstream filing is complete: docker volume `harness_ldk-server-data`, `/data/regtest/ldk_node_data.sqlite.bak-lsps2`.

Full evidence/runbook: `explore-lsp-restart-issue.txt`.
