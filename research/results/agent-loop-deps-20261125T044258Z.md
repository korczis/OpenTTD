# agent-loop-deps, 2026-11-25T04:42:58Z

RESEARCH-ONLY. `tools/research/agent-loop-deps` (I0171): the cross-repository pieces of the decision path ([agent loop](../agent-loop.md)). Every line is OBSERVED by this run.

```
OBSERVED echo      NOT RUN  Prismatic echoes observed_digest in GET /api/openttd/decision
  evidence  GET http://127.0.0.1:4004/api/openttd/decision: unreachable ([Errno 61] Connection refused); the echo was not observed
  main      prismatic-platform origin/main b6df70205d: no observed_digest in apps/prismatic_api/lib/prismatic_api/controllers/openttd_controller.ex; the decision is answered at apps/prismatic_api/lib/prismatic_api/controllers/openttd_controller.ex:50
  request   prismatic-platform: echo `observed_digest`, the digest of the companies it read from OPENTTD_BRIDGE_URL, in the answer of GET /api/openttd/decision (OpenttdController#decision, apps/prismatic_api/lib/prismatic_api/controllers/openttd_controller.ex); this fork's panel already checks an echo (tools/research/mcp-rhai/src/panel_decision.rs `freshness`)
OBSERVED approval  missing  the bridge's main holds feature/operator-approval
  evidence  09afefa is not an ancestor of openttd-prismatic-bridge origin/main dd9119b16f
  evidence  no POST /decision/resume in bridge/main.py
  evidence  the bridge approves its own request at bridge/decision.py:151: approved = await rcon(f"prismatic_mcp approve {approval}")
  note      checkout feat/live-behaviour 513398c: 1 ahead of, 17 behind origin/main
  request   openttd-prismatic-bridge: merge feature/operator-approval (09afefa: the bridge approves nothing itself, a waiting setting is answered `pending`, POST /decision/resume applies it once the operator approved it in the game) into main
OBSERVED companies NOT RUN  the bridge's GET /companies carries a tick and a digest
  evidence  GET http://127.0.0.1:8730/companies: unreachable ([Errno 61] Connection refused); tick and digest were not observed
  main      openttd-prismatic-bridge origin/main dd9119b16f: GET /companies at bridge/main.py:236; its handler names no tick and no digest
  request   openttd-prismatic-bridge: carry the game tick and a digest of the companies' identities on GET /companies, as this fork's panel records them (tools/research/mcp-rhai/src/panel_decision.rs `observe`, `identity_digest`)
agent-loop-deps: 0 met, 1 missing, 2 NOT RUN (missing, exit 1)
```
