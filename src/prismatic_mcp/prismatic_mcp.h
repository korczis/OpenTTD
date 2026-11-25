/*
 * This file is part of korczis's PRIVATE RESEARCH FORK of OpenTTD and is NOT
 * part of upstream OpenTTD. See research/prismatic-mcp/ and AGENTS.md.
 *
 * RESEARCH-ONLY, UNSTABLE, DEBUG-ONLY. Compiled only when the CMake option
 * OPTION_PRISMATIC_MCP_SERVER=ON (which itself requires
 * OPTION_RESEARCH_INSTRUMENTATION=ON) defines OTTD_PRISMATIC_MCP_SERVER.
 */

/** @file prismatic_mcp.h Embedded, loopback-only MCP server for OpenTTD (research-only). */

#ifndef PRISMATIC_MCP_H
#define PRISMATIC_MCP_H

#ifdef OTTD_PRISMATIC_MCP_SERVER

#include <cstdint>
#include <span>
#include <string>
#include <string_view>
#include <vector>

#include "prismatic_mcp_rules.h"

/**
 * Lifecycle + control surface of the embedded MCP server. Every function here
 * runs on the OpenTTD main (game) thread; the server never touches game state
 * from any other thread. All entry points are no-ops unless the server has been
 * started. See research/prismatic-mcp/04-architecture-decision-record.md.
 */
namespace PrismaticMCP {

/** MCP protocol revision this server pins to (stable spec). */
inline constexpr std::string_view PROTOCOL_VERSION = "2025-11-25";
/** Default loopback port for the Streamable-HTTP endpoint. */
inline constexpr uint16_t DEFAULT_PORT = 8731;

/** Called once from openttd_main after NetworkStartUp(); allocates nothing heavy. */
void Init();
/** Called once from ShutdownGame(); stops the listener and clears the token. */
void Shutdown();
/** Called every frame from GameLoop() on the main thread; services the listener. */
void Poll();
/** Invalidate cached world snapshots on a game-mode transition (new/load/menu). */
void OnGameModeChanged();

/** Start the loopback listener on @p port (0 = DEFAULT_PORT). Returns false on error. */
bool Start(uint16_t port, std::string &error);
/** Stop the listener; idempotent. */
void Stop();
/** True while the listener is bound and accepting. */
bool IsRunning();

/** The full endpoint URL, e.g. "http://127.0.0.1:8731/mcp" (empty if stopped). */
std::string GetEndpoint();
/** The current bearer token in hex (empty if stopped). For the local operator only. */
std::string GetToken();
/** Generate a fresh token and invalidate every existing session. */
void RotateToken();
/** One machine-readable status line for the console (never contains the token). */
std::string GetStatusLine();

/* --- approval workflow (operator surface for the console) ------------------ */
/** Human-readable list of pending approvals (id, tool, args). Never a token. */
std::string ListApprovals();

/**
 * One still-PENDING approval, as the operator console needs it: the id to act on
 * and exactly what consenting to it would do. Never a token.
 */
struct PendingApprovalInfo {
	uint64_t id = 0;
	std::string tool;            ///< the gated action this approval belongs to (ai.set_setting, command.BuildRail).
	std::string canonical_args;  ///< the exact arguments the approval binds to.
	bool expired = false;        ///< past its TTL: listed, but ApproveById will refuse it.
	std::string mcp_tool;        ///< the MCP tool as it was called (openttd.ai.set_setting).
	uint64_t matched_rule = 0;   ///< the standing rule that applies to it (Rules::Decide), else 0.
	bool rule_decides = false;   ///< whether that rule may decide it (false: an allow rule held back, or it expired).
};
/**
 * Every approval whose state is still Pending, in id order. The console, the control window and
 * openttd.ui.control read it; it decides nothing. Consent stays the owner's act (ADR 0038, ADR
 * 0040): one id at a time (ApproveById), or in bulk only through DecideAll and the owner's standing
 * rules (AddApprovalRule), which the game's console, the owner's own click in the control window, or
 * the panel's owner-authenticated routes make, never an MCP client, a script or injected input.
 */
std::vector<PendingApprovalInfo> PendingApprovals();

/* --- bulk consent and standing rules (ADR 0041) ------------------------------------------ */

/** An approval's id with why it was not decided as asked. */
struct IdError {
	uint64_t id = 0;   ///< the approval.
	std::string error; ///< why: the core's own message.
};

/** What a bulk decision, a list of ids or a new rule did, one console line per approval it decided or skipped. */
struct BulkOutcome {
	bool ok = true;                 ///< false: refused as a whole (a malformed rule, no such rule); lines say why.
	uint64_t rule_id = 0;           ///< the rule added, for AddApprovalRule.
	int decided = 0;                ///< approvals approved (and executed) or denied.
	int refused = 0;                ///< approvals not decided as asked: refused before they ran, or run and rejected.
	int skipped = 0;                ///< approvals the filter took but this channel may not decide (expired, owner-only).
	std::vector<std::string> lines; ///< what happened, in order: what each id did, its tool and its arguments.
	std::vector<uint64_t> decided_ids; ///< the ids decided, for the machine-readable result line.
	std::vector<IdError> refused_ids;  ///< refused before anything ran (not pending, expired, not found, owner-only).
	std::vector<IdError> rejected_ids; ///< approved and run, but the command was refused (nothing enacted).
	std::vector<IdError> skipped_ids;  ///< selected but not taken (expired, owner-only for this channel).
};

/**
 * The machine-readable result line a console command prints when it carries --nonce (ADR 0041):
 * `[approval-core] {"nonce", "ok", "decided": [ids], "refused": [{id, error}], "rejected": [{id, error}],
 * "skipped": [{id, error}], "rule_id", "error"}`. The panel reads only this line.
 * @param nonce The caller's token.
 * @param out What the command did.
 * @param error Why the command failed as a whole; empty when it did not.
 * @return The line.
 */
std::string ResultLine(std::string_view nonce, const BulkOutcome &out, std::string_view error);

/**
 * Why an unattended channel ("auto", "file") may not approve pending approval \p id, by #264's
 * condition: one company's own action, for a company a registered approver played when it was
 * asked and plays now. Empty when it may, for any other channel, and for an id not pending.
 */
std::string UnattendedHoldOf(uint64_t id, std::string_view via);

/** Whether approval \p id was approved and run (its command posted or refused), not merely pending or denied. */
bool HasRun(uint64_t id);

/**
 * Count what one id did into a bulk outcome: decided; approved and run but rejected; or refused before anything ran.
 * @param out The outcome counted into.
 * @param id The approval.
 * @param was_pending Whether it was pending before it was asked: only then can a refusal be a rejected run.
 * @param ok Whether ApproveById or DenyById succeeded.
 * @param msg Their message.
 */
void Tally(BulkOutcome &out, uint64_t id, bool was_pending, bool ok, const std::string &msg);
/** Every standing rule, in id order. */
std::vector<Rules::Rule> ApprovalRules();
/**
 * Add a standing rule and apply it at once to every approval already waiting (a deny rule withdraws
 * what it matches; an allow rule approves and executes it, unless it expired or the rule may not
 * decide it). From then on every new approval is checked against the rules as it is created. A
 * spec with `persist` is written to the rules file (RulesFile(), mode 0600) and loaded at the next
 * start. `spec.via` is recorded as created_via; a rule made through the panel never decides an
 * owner-only tool.
 */
BulkOutcome AddApprovalRule(Rules::Effect effect, const Rules::Spec &spec);
/** Remove rule \p id (and from the rules file when it was persisted); false when there is none. */
bool RemoveApprovalRule(uint64_t id, std::string &message);
/**
 * Approve all (\p approve) or deny all pending approvals the spec's tool glob and matcher select.
 * Approving skips an expired one and, for a spec from the panel, an owner-only one; each approval
 * still goes through ApproveById (or DenyById) on its own and records `spec.via` as its channel.
 */
BulkOutcome DecideAll(bool approve, const Rules::Spec &spec);
/** The file standing rules made with --persist are kept in: one per server port, in the personal directory. */
std::string RulesFile();
/**
 * Load the persisted rules of RulesFile(), once per file, and say each one on the console and in
 * the log: a standing rule is never silent. Start() calls it.
 * @return The lines said: one per rule loaded, and one per entry or file that could not be read.
 */
std::vector<std::string> LoadApprovalRules();

/** Approve a pending mutation by id and carry it out at once; false if not found, expired or not
 * executed. `via` names the channel, recorded with its decision (approved_via). */
bool ApproveById(uint64_t id, std::string &message, std::string_view via = "console");
/** Whether this process's own MCP server holds a pending approval with this id. */
bool HoldsPending(uint64_t id);
/**
 * The approval channel from inside the game (ADR 0038), on the server: a chat line from client
 * `client_id` playing `company` (a spectator's is no company) with public key `key`.
 * '!approvals [page]' answers the company's pending approvals, one machine-readable line each
 * ("[approvals] {json}") and a last count line; '!approve <id>' and '!deny <id>' (one id per line)
 * decide through ApproveById/DenyById when detail::DecideChat allows it: an approval naming the
 * client's company, of a company-scoped action, approved only by a key the server's operator
 * registered (never by a company's allow list), and not from injected input. `replies` go to the asker alone, each within
 * the chat length.
 * @return false when the line is not one of these (it is chat as usual).
 */
bool ChatApproval(uint32_t client_id, int company, std::string_view key, bool injecting, std::string_view line, std::vector<std::string> &replies);
/** The approvers the server's operator registered (client public keys), persisted in the personal directory. */
std::vector<std::string> Approvers();
/** Register an approver's key; false when it is empty or already there. */
bool AddApprover(std::string_view key);
/** Remove the one approver whose key starts with this (at least 8 characters, matching exactly one
 * key); the key removed, or empty when none or several match. */
std::string RemoveApprover(std::string_view key_or_prefix);
/** Deny a pending mutation by id; `via` names the channel, recorded as how it was settled (settled_by). */
bool DenyById(uint64_t id, std::string &message, std::string_view via = {});
/**
 * The approval deadline: after `days` game days unanswered (0: off) a pending approval is settled
 * with the operator's standing verdict, "keep" (retire it, enacting nothing; the default) or
 * "approve" (execute it when a chat approver could decide it, else retire it). False and a usage
 * line in `message` for a malformed policy; else the policy now in force.
 */
bool SetApprovalDeadline(std::string_view days, std::string_view verdict, std::string &message);
/** The approval deadline in force, as one console line. */
std::string DescribeApprovalDeadline();
/** Machine-readable summary of the decision/provenance trace for the console. */
std::string DecisionTrace(int max_rows);

/* --- the catalogue as console commands (prismatic_mcp_console.cpp) ------- */

/** `mcp`: every tool and resource of the server as a console command, derived from its lists. */
bool ConMcp(std::span<std::string_view> argv);

} // namespace PrismaticMCP

#endif /* OTTD_PRISMATIC_MCP_SERVER */
#endif /* PRISMATIC_MCP_H */
