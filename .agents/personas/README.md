# .agents/personas/ — role guides for specialized agents

Predefined, repo-specific role briefs. Use them when you spawn a sub-agent for
a focused task (e.g. a code review, a Rust change, a dashboard change, a docs
pass) so it behaves consistently without re-deriving conventions.

Each persona is a short markdown brief: the **context** the role needs, the
**conventions/rules** it must follow, and the **verification** it should do
before calling a change done.

| persona | use for |
|---|---|
| `rust-backend.md` | any change to `src/` (server, routes, auth, perms, config, metrics, tls) |
| `dashboard-ts.md` | any change to `src/assets/ts/` (dashboard SPA) + jsdom harnesses |
| `docs-writer.md` | any change to `docs/`, `README.md`, or `.agents/knowledge/` |
| `security-reviewer.md` | security review of auth/perms/throttling/input handling |

> Interaction with pi's own agent system: pi picks up *custom* agent types from
> `.pi/agents/*.md` (project) / the global agent dir. The personas here are
> **repository conventions**, not pi agent types; if you want a persona to also
> be spawnable as a pi Agent, mirror the relevant parts into a `.pi/agents/`
> file. See `.agents/settings/README.md` for the companion config folder.
