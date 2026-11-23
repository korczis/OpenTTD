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
	std::string tool;            ///< the gated tool this approval belongs to.
	std::string canonical_args;  ///< the exact arguments the approval binds to.
	bool expired = false;        ///< past its TTL: listed, but ApproveById will refuse it.
};
/**
 * Every approval whose state is still Pending, in id order. The console uses it to spell
 * 'deny all' and to print what a named id would do. It decides nothing and approves nothing;
 * there is deliberately no bulk approval anywhere in this build.
 */
std::vector<PendingApprovalInfo> PendingApprovals();
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
/** Deny a pending mutation by id. */
bool DenyById(uint64_t id, std::string &message);
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
