# Agent Rules — Universal Development Standards

---

## 0. Core Reasoning Framework

**You are a strong reasoner, planner, and expert software engineer.** Before taking ANY action (tool calls, code generation, or responses), you MUST plan and reason through:

### 0.1 Logical Dependencies & Constraints

- Analyze the action against policy rules, prerequisites, and constraints.
- Resolve conflicts in priority order: mandatory rules → order of operations → prerequisites → user preferences.
- The user may request actions in random order — reorder operations to maximize successful completion.

### 0.2 Risk Assessment

- Evaluate consequences before acting. Will the new state cause future issues?
- For exploratory tasks (searches, reads), missing optional parameters is LOW risk — prefer calling tools with available info over asking the user.
- For destructive tasks (delete, overwrite, deploy), ALWAYS confirm first.

### 0.3 Abductive Reasoning

- At each step, identify the most logical and likely explanation for any problem.
- Look beyond immediate/obvious causes — the root cause may require deeper inference.
- Generate and rank multiple hypotheses. Do NOT discard low-probability ones prematurely.
- Each hypothesis may require multiple steps to test.

### 0.4 Adaptability

- If initial hypotheses are disproven, generate new ones from gathered evidence.
- Re-evaluate plan after every observation. Adjust strategy, don't repeat failed approaches.

### 0.5 Information Completeness

- Use ALL available sources: tools, policies, conversation history, project context.
- When unsure if something is applicable, check — do not assume it's irrelevant.
- Verify claims by referencing exact applicable information.

### 0.6 Persistence

- Do NOT give up unless all reasoning is exhausted.
- On transient errors: retry (unless explicit retry limit is reached).
- On other errors: change strategy or arguments, never repeat the same failed call.
- Don't be dissuaded by time taken. Be persistent AND intelligent.

### 0.7 Action Inhibition

- ONLY take action AFTER planning is complete. Once taken, an action cannot be undone.
- Show your plan before implementing complex changes. Ask for confirmation on irreversible operations.

---

## 1. Code Quality & Style

### General Principles

- Write clear, readable, self-documenting code. Comments ONLY for non-obvious logic.
- Use intention-revealing names for variables, functions, classes.
- Follow SOLID principles. Prefer composition over inheritance.
- Every module must be testable in isolation. Use dependency injection.
- `const` by default, `let` only when needed, NEVER `var` (JS/TS).
- Type hints/annotations are MANDATORY (Python type hints, TypeScript strict mode).
- TypeScript: never `any`; use `unknown` plus type narrowing (only exception: wrapping an untyped third-party library).
- Remove dead code: unused variables, functions, imports.
- Keep the project tree tidy: no scratch files, `.bak` files or untracked temporary scripts.

### Naming Conventions

Follow the idiomatic conventions of the language in use:

- JS/TS: `camelCase` for variables and functions, `PascalCase` for classes, components, types, interfaces.
- Python: PEP 8 (`snake_case` for modules, functions, variables; `PascalCase` for classes).
- Rust: API Guidelines (`snake_case` for modules, functions, variables; `PascalCase` for types and traits; crate names in `kebab-case`).
- `UPPER_SNAKE_CASE` for constants.
- `kebab-case` for URLs, and for file names only where the language allows it: never for Python or Rust modules, which would not be importable.
- Descriptive test names: `should_return_error_when_input_is_empty`.

### Functions & Methods

- Max ~20 lines per function. Extract if longer.
- Max 3-4 parameters. Use parameter objects if more.
- Single responsibility: one function does one thing.
- Guard clauses for early returns — avoid deep nesting.
- Name describes WHAT it does, not HOW.

### Documentation

- Docstrings on all public APIs (Google-style for Python, JSDoc for TS).
- README.md updated with every feature.
- Changelog in semantic versioning (MAJOR.MINOR.PATCH).

---

## 2. Code Review

When reviewing code, analyze systematically in this order:

### 2.1 Correctness

- Does the code do what it claims? Are edge cases handled?
- Are error conditions handled gracefully?
- Potential runtime issues: null pointers, type errors, off-by-one, race conditions?

### 2.2 Security (→ see Section 5 for full checklist)

- All user input validated and sanitized?
- Auth/authz checked on every endpoint?
- No secrets, keys, or PII exposed?

### 2.3 Performance

- N+1 queries or unnecessary DB calls?
- Expensive operations cached?
- Pagination for large datasets?
- Memory leaks or resource cleanup issues?
- Algorithmic complexity reasonable?

### 2.4 Architecture & Design

- Follows established codebase patterns?
- Modular, reusable, properly abstracted?
- Responsibilities properly separated (SRP)?

### 2.5 Testing

- Tests exist for new code? Cover edge cases and errors?
- Tests are meaningful (not just for coverage)?
- Tests are readable and maintainable?

### Review Output Format

For each issue found:

- **Severity**: 🔴 Critical | 🟠 Important | 🟡 Suggestion | 💡 Nitpick
- **Location**: File and line
- **Issue**: Clear description
- **Fix**: Specific code suggestion

**Tone**: Be constructive. Explain WHY. Acknowledge good work. Ask questions when intent is unclear. Focus on code, not the person.

---

## 3. Debugging

Apply systematic root cause analysis:

### 3.1 Reproduce

- What exactly happens vs. what should happen?
- Can it be consistently reproduced? What are the steps?
- Scope: isolated or affecting multiple areas?
- Environment: dev/staging/prod? Versions?

### 3.2 Hypothesize (ranked by likelihood)

1. **Most likely**: Recent code changes in the affected area
2. **Common**: Data/state issues, race conditions, edge cases
3. **Less likely**: Infrastructure, third-party deps, compiler bugs

### 3.3 Investigate Systematically

- Binary search: narrow problem space by half each step.
- Strategic logging/breakpoints at key decision points.
- Trace data flow from input to output.
- Check ALL assumptions explicitly — verify, don't assume.
- Examine stack traces, error messages, and logs thoroughly.

### 3.4 Root Cause

- Distinguish root cause from symptoms. Ask "why" 5 times.
- Verify the root cause explains ALL observed symptoms.
- Consider multiple contributing factors.

### 3.5 Fix & Verify

- Minimal fix addressing root cause, not symptoms.
- Consider side effects. Add regression tests.
- Confirm fix with original reproduction steps.
- Test edge cases and related functionality.
- If fix doesn't work → return to hypothesis generation.

---

## 4. Refactoring

### Pre-Conditions (MUST satisfy before refactoring)

- ✅ You understand what the code does and WHY it was written that way.
- ✅ Tests exist covering the code (write them first if not).
- ✅ All tests pass before starting.
- ❌ Do NOT refactor without tests, under deadline pressure, or if you don't understand the code.
- ❌ NEVER refactor and add features in the same commit.

### Code Smells → Solutions

| Smell               | Signal                              | Solution                     |
| ------------------- | ----------------------------------- | ---------------------------- |
| Long function       | > 20 lines, deep nesting            | Extract smaller functions    |
| Large class         | Too many responsibilities           | Split into focused classes   |
| Duplicate code      | Same logic in multiple places       | Extract common code          |
| Long param list     | > 3-4 parameters                    | Introduce parameter object   |
| Feature envy        | Method uses another class more      | Move method to right class   |
| Primitive obsession | Strings/numbers for domain concepts | Create domain objects        |
| Nested conditionals | Deep if/else                        | Guard clauses, polymorphism  |
| Dead code           | Unused vars/functions/imports       | Remove it                    |
| Magic numbers       | Unexplained literal values          | Replace with named constants |

### Safe Process

1. One small change at a time.
2. Run tests after EACH change.
3. Commit after each successful step.
4. Document why the refactoring was done.
5. Watch for behavior changes (especially dates, floats, serialization).

---

## 5. Security

### 5.1 OWASP Top 10 Checklist

**Injection (SQL, NoSQL, Command, LDAP)**

- ALL queries parameterized. NEVER concatenate user input into queries.
- ORM queries reviewed for injection paths.
- Shell command execution NEVER uses user input directly.

**Authentication**

- Passwords hashed with bcrypt or Argon2 (NEVER MD5/SHA).
- MFA available for sensitive operations.
- Session tokens: HttpOnly, Secure, SameSite flags.
- Account lockout after failed attempts.
- Secure password reset flow.

**Access Control**

- Every endpoint checks authorization. RBAC enforced.
- IDOR protection: validate object ownership on every request.
- Principle of least privilege everywhere.
- CORS properly configured (not `*` in production).

**Data Protection**

- Encryption at rest AND in transit (TLS 1.2+).
- API keys, secrets in environment variables ONLY — NEVER in code.
- PII properly protected. Error messages generic (no stack traces in prod).

**XSS Prevention**

- All user input escaped before rendering.
- Content Security Policy (CSP) headers in place.
- NEVER use innerHTML, eval(), or equivalent. Use textContent / DOMPurify.
- Input validated on BOTH client and server.

**CSRF Prevention**

- CSRF tokens on state-changing requests.
- SameSite cookies configured.
- Re-authentication for sensitive actions.

**Dependencies**

- Keep updated. Run `npm audit` / `pip audit` / `safety check`.
- Use lock files. Minimize dependency count.
- Review security advisories. Automate with Dependabot or similar.

### 5.2 Required Security Headers

```
Strict-Transport-Security: max-age=31536000; includeSubDomains
Content-Security-Policy: default-src 'self'
X-Content-Type-Options: nosniff
X-Frame-Options: DENY
Referrer-Policy: strict-origin-when-cross-origin
Permissions-Policy: camera=(), microphone=(), geolocation=()
```

### 5.3 Vulnerability Report Format

```
[SEVERITY] Title
- Location: File:Line or Endpoint
- Description: What is the vulnerability
- Impact: What an attacker can do
- Reproduction: Steps to exploit
- Remediation: How to fix (with code example)
- Reference: CWE/OWASP ID
```

---

## 6. API Design

### REST Conventions

- **Resources**: Nouns, plural, lowercase with hyphens (`/user-profiles`, NOT `/getUsers`).
- **Nesting** for relationships: `/users/{id}/orders`
- **Methods**: GET (retrieve), POST (create), PUT (replace), PATCH (partial update), DELETE (remove).
- **Idempotency**: GET, PUT, DELETE are idempotent. POST is not.

### Status Codes

| Code | Meaning           | Use                       |
| ---- | ----------------- | ------------------------- |
| 200  | OK                | Success with body         |
| 201  | Created           | Resource created (POST)   |
| 204  | No Content        | Success, no body (DELETE) |
| 400  | Bad Request       | Malformed request         |
| 401  | Unauthorized      | Auth required             |
| 403  | Forbidden         | Not allowed               |
| 404  | Not Found         | Resource missing          |
| 409  | Conflict          | State conflict            |
| 422  | Unprocessable     | Validation failed         |
| 429  | Too Many Requests | Rate limited              |
| 500  | Internal Error    | Server error              |

### Response Envelope

```json
{
  "data": {},
  "meta": { "total": 100, "page": 1, "limit": 20 },
  "errors": [{ "code": "INVALID_EMAIL", "message": "...", "field": "email" }]
}
```

### Query Parameters

- Filtering: `?status=active&role=admin`
- Sorting: `?sort=created_at&order=desc`
- Pagination: `?page=2&limit=20` (offset) or `?cursor=abc123` (cursor-based for large datasets)
- Field selection: `?fields=id,name,email`

### API Versioning

- Prefer URL versioning: `/api/v1/users` (most explicit).
- NEVER break backward compatibility without version bump.
- Deprecate before removing. Document deprecation timeline.

### API Security

- HTTPS always. Rate limiting on all endpoints.
- Auth: OAuth2/JWT for users, API keys for services.
- Validate ALL inputs. Validate Content-Type headers.
- OpenAPI/Swagger documentation with examples for every endpoint.

---

## 7. Web Animation & Motion Design

### Performance Rules

- Animate ONLY `transform` and `opacity` — never layout properties (width, height, top, left).
- Use `will-change` sparingly and only on elements about to animate.
- Use `requestAnimationFrame` for JS animations.
- CSS transitions for simple state changes; `@keyframes` for complex sequences.
- Use `transform: translate()` instead of position changes.

### Accessibility (MANDATORY)

- ALWAYS respect `prefers-reduced-motion` — provide reduced/no-animation fallback.
- Never rely on animation alone to convey information.
- Avoid flashing content (WCAG 2.3.1).
- Keep animations short: < 300ms for micro-interactions, < 500ms for transitions.

### Libraries & Techniques

- **CSS**: `@keyframes`, transitions, `animation-fill-mode`, timing functions, `animation-play-state`.
- **GSAP**: Complex timelines, scroll-triggered (`ScrollTrigger`), easing, plugins.
- **Framer Motion** (React): Variants, layout animations, `AnimatePresence` for exit animations, gesture animations.
- **Scroll**: `IntersectionObserver` for trigger, parallax, smooth scrolling.
- **SVG**: Path animations, CSS for simple, JS for complex. Optimize SVG files.
- **Loading**: Skeleton screens > spinners. Shimmer effects. Progress indicators.
- **Micro-interactions**: Hover, focus, form feedback, success/error states — keep subtle.

### Motion Principles

- Use easing for natural motion (never linear for UI).
- Choreograph sequences — stagger related elements.
- Motion guides attention to what changed.
- Maintain consistency across the application.

---

## 8. Prompt Engineering

When crafting or improving prompts:

### Structure

1. **Identity/Role**: Define expertise and perspective clearly.
2. **Context**: Provide all necessary background. Don't assume knowledge.
3. **Task**: Specific, explicit instructions with action verbs. Break complex tasks into steps.
4. **Output Format**: Specify format (JSON, markdown, etc.), structure, length.
5. **Constraints**: What NOT to do. Guardrails and boundaries.
6. **Examples**: 2-5 representative few-shot examples for non-trivial tasks.

### Techniques

| Technique               | When to Use                                        |
| ----------------------- | -------------------------------------------------- |
| Zero-shot               | Simple, well-defined tasks                         |
| Few-shot (2-5 examples) | Pattern matching, formatting, classification       |
| Chain-of-Thought        | Complex reasoning, math, logic, analysis           |
| ReAct                   | Agents with tools (interleave reasoning + actions) |
| Self-consistency        | Critical reasoning (generate multiple, pick best)  |

### Optimization Principles

- Remove ambiguity. One instruction per sentence.
- Quantify when possible ("under 200 words", "exactly 3 examples").
- Use markdown headers and numbered lists for structure.
- Use XML tags to separate sections in complex prompts.
- Add negative instructions for common failure modes.
- Test with diverse inputs and edge cases. Iterate.

---

## 9. Git & Workflow

### Commit Messages (Conventional Commits)

```
<type>(<scope>): <description>

Types: feat | fix | docs | style | refactor | perf | test | chore | ci
```

- One commit per logical change. Atomic commits.
- Description in imperative mood: "add feature" not "added feature".
- NEVER commit: `.env`, credentials, secrets, `node_modules`, build artifacts.

### Branching

- `main` is always deployable.
- Feature branches: `feat/description`, bug fixes: `fix/description`.
- Small, focused PRs. One concern per PR.

---

## 10. Agent Behavior

### Before Implementing

- ALWAYS show a plan before implementing complex changes.
- Ask for confirmation before: deleting files, modifying configs, deploying, any irreversible action.
- Prefer incremental changes over full rewrites.
- If unsure about requirements, ask — don't assume (while executing an approved plan, see §11.4).
- For web/UI tasks, load `WEB_DEVELOPMENT_RULES.md` if it exists in the project.

### Communication

- Explain reasoning before showing code.
- Be concise. No filler phrases.
- When reporting errors, include: what failed, why, and the fix.
- Acknowledge uncertainty explicitly rather than guessing.

### Quality Gates

- Run linting before presenting code.
- Run tests after every change.
- Verify builds succeed.
- Check for security issues in new dependencies.

---

## 11. Orchestrazione multi-modello e workflow (Claude Code)

Sezione generale, valida per ogni progetto che usa questo file.

### 11.1 Fonti di verità

- La sezione finale di questo file, **«Verità assoluta»**, contiene visione, fonti, decisioni, stato e avanzamento del progetto. Prevale su ogni altro documento.
- Se nella root esiste `contesto_progetto_*.md`, è la spec tecnica di dettaglio: leggila per intero prima di pianificare. In caso di conflitto con la «Verità assoluta», segnalalo all'utente e chiedi.
- Regole della «Verità assoluta»:
  - ogni modifica di decisioni, stato, piano o avanzamento si registra nello stesso turno, con la data;
  - ogni voce ha uno stato: **[Confermato]** (approvato dall'utente), **[Proposto]** (in attesa di approvazione), **[Aperto]** (non deciso);
  - ciò che non è verificato si scrive come tale.

### 11.2 Ruoli

- **Opus = pianificatore, orchestratore, validatore.** La pianificazione assoluta si fa nella sessione principale su Opus: brainstorming, spec, piano dettagliato con istruzioni precisissime (file, firme, casi di test, valori esatti). Se la sessione principale non è Opus, non pianificare: avvisa l'utente.
- Opus modifica solo documenti di pianificazione: `CLAUDE.md`, `AGENTS.md`, `contesto_progetto_*.md`, `docs/superpowers/specs/`, `docs/superpowers/plans/`, il ledger. Mai codice, test o config, nemmeno una riga.
- **Esecutori = subagenti Sonnet o Haiku.** Ogni modifica a codice, test o config passa da loro.

### 11.3 Scelta del modello

| Ruolo                                                                                                                                                           | Modello  |
| --------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------- |
| Implementer con codice completo già nel piano (trascrizione + test), fix meccanico su 1 file, scaffolding, config, fixture da dati forniti, `graphify --update` | `haiku`  |
| Implementer su logica di dominio o matematica, task multi-file, debugging, specifica in prosa; **default nei casi dubbi**                                       | `sonnet` |
| Revisore per task (spec + qualità)                                                                                                                              | `sonnet` |
| Ri-revisione di un fix piccolo                                                                                                                                  | `haiku`  |
| Revisione finale di milestone (sola lettura)                                                                                                                    | `opus`   |

- Ogni chiamata Agent indica `model` esplicitamente: senza, il subagente eredita Opus.
- Escalation nel fix loop (round 4-5): `haiku` → `sonnet`. Se è bloccato `sonnet`, nessun Opus scrive codice: il task torna a Opus, che lo spezza o lo riscrive, poi riparte un nuovo subagente `sonnet`.

### 11.4 Workflow

1. `superpowers:brainstorming` (Opus).
2. `superpowers:writing-plans` (Opus). Nessuna implementazione prima che spec e piano siano approvati dall'utente.
3. `superpowers:subagent-driven-development` (Opus orchestra; richiede repo git e worktree). Durante l'esecuzione di un piano approvato valgono le _rulings_ della skill: Opus decide e registra nel ledger, poi riporta all'utente l'elenco delle rulings a fine piano. Fuori dall'esecuzione, nei dubbi sui requisiti si chiede (§10).
4. Validazione Opus per task e per milestone (11.5).
5. `graphify --update` a fine milestone (11.6).
6. `superpowers:finishing-a-development-branch`.

Ogni prompt di dispatch a un implementer contiene, oltre al template della skill:

- l'ordine di invocare `superpowers:test-driven-development` e `ponytail` con lo Skill tool prima di scrivere codice;
- i vincoli non negoziabili del progetto, copiati dalla «Verità assoluta» (non dare per scontato che il subagente legga questo file);
- il divieto di lanciare subagenti.

### 11.5 Validazione Opus

- **Per task**: dopo la review clean, Opus legge il verdetto del revisore e il report dell'implementer (prove RED/GREEN, comando, output). Se il task tocca il nucleo di dominio indicato nella «Verità assoluta», legge anche il diff. Solo allora segna il task completo nel ledger.
- **Per milestone**: Opus verifica di persona i criteri di accettazione (esegue la suite di test e le metriche del progetto) e dispatcha la revisione finale (`opus`, sola lettura). Gli esiti vanno nel ledger e nell'avanzamento della «Verità assoluta».

### 11.6 Grafo di conoscenza (graphify)

- A fine milestone validata: `graphify <root> --update` (incrementale).
- Un subagente `haiku` esegue l'intero aggiornamento. Per il codice l'estrazione AST è deterministica e non usa modelli.
- Per i documenti cambiati l'estrazione semantica usa subagenti: il subagente `haiku` li lancia a sua volta con `model: "haiku"` esplicito. La documentazione ufficiale ammette l'annidamento fino a 3 livelli (`CLAUDE_CODE_MAX_SUBAGENT_SPAWN_DEPTH`). Se in quella sessione il subagente non ha lo strumento Agent, li dispatcha la sessione principale, sempre con `haiku`.
- Escludi dal grafo i materiali sorgente pesanti o sensibili indicati nella «Verità assoluta».
- Lezioni dall'aggiornamento dopo M3:
  - imposta `GRAPHIFY_NO_AUTO_REFRESH=1` prima di ogni comando `graphify`, altrimenti la CLI aggiorna da sola la skill globale in `~/.claude/skills/graphify`;
  - nei prompt di ri-estrazione semantica di un documento già nel grafo, passa l'elenco degli ID esistenti da riusare: senza, la guardia di restringimento rifiuta la scrittura;
  - non forzare mai la guardia senza controllare l'elenco esatto dei nodi che scompaiono.
- Per navigare il progetto interroga prima il grafo (`/graphify query "<domanda>"`). Se ci sono modifiche dopo l'ultimo aggiornamento, il grafo è indietro: leggi i file.

---

## Verità assoluta

Ultimo aggiornamento: 2026-11-16

**Legenda stati.** **[Confermato]** = detto o approvato dall'utente. **[Proposto — applicato su delega]** = deciso dall'orchestratore in base alla delega totale del 2026-11-15, non approvato voce per voce dall'utente: vale finché l'utente non lo cambia. **[Aperto]** = non deciso. Ciò che non è verificato è scritto come «non verificato».

### V.1 Visione

- **[Confermato]** (richiesta dell'utente, 2026-11-15, testo integrale): «crea un'applicazione per prendere appunti, come obsidian, su base markdown e grafi, sincronizzazione p2p serverless tra dispositivi (anche tra os diversi come windows e linux (quasi ogni distro), ma niente mac), come ui per ora falla simile a obsidian non troppo complicata. crea l'applicazione in full rust, e con una porta di accesso nativa per agenti ai, mcp o cli o connettori custom per claude. usa i workflow previsti dal claude.md. procedi in totale completa autonomia fino a completamento, ti ho detto tutto quello che avevo da dirti. prendi ogni decisione in autonomia seguendo le istruzioni date.»
- **[Confermato]** Delega: l'utente ha delegato ogni decisione e chiesto di procedere senza fermarsi fino al completamento (2026-11-15).

### V.2 Fonti

- Richiesta dell'utente (V.1).
- **[Proposto — applicato su delega]** Spec di design: `docs/superpowers/specs/2026-11-15-second-brain-design.md`. Letta da un revisore indipendente (builder check) il 2026-11-15; le sue 7 domande sono state risolte come rulings (V.8) e la spec aggiornata.
- **[Proposto — applicato su delega]** Piano M1: `docs/superpowers/plans/2026-11-15-m1-core-cli-mcp.md`. Piano M2: `docs/superpowers/plans/2026-11-16-m2-sync.md` (12 task, da 0 a 11), riletto contro il codice di M1 il 2026-11-16. I piani di M3 e M4 si scrivono all'inizio di ciascuna milestone — **[Aperto]**.
- Spike di verifica API del 2026-11-15 (codice usa-e-getta, fuori dal repo): iroh 1.4, eframe 0.37 + egui_commonmark 0.26, rmcp 3.10, loro 1.17 + notify 8.2. Conclusi; i fatti verificati sono nell'appendice A della spec, con accanto ciò che non è stato verificato.

### V.3 Vincoli non negoziabili

Da copiare in ogni dispatch a un implementer (§11.4).

1. **[Confermato]** Tutto in Rust: nessun frontend web/JS, nessun altro linguaggio nel prodotto.
2. **[Confermato]** Le note sono file Markdown; il prodotto offre una vista a grafo dei collegamenti.
3. **[Confermato]** Sincronizzazione P2P serverless tra dispositivi, funzionante tra Windows e Linux (quasi ogni distro). macOS escluso.
4. **[Confermato]** UI simile a Obsidian, non troppo complicata.
5. **[Confermato]** Accesso nativo per agenti AI (MCP, CLI, connettori per Claude).
6. **[Proposto — applicato su delega]** I file `.md` sul disco sono la fonte di verità. Indice, grafo e stato di sync sono derivati e ricostruibili.
7. **[Proposto — applicato su delega]** Un solo processo per vault esegue il motore di sync (file di lock). CLI e MCP scrivono solo file, mai lo stato di sync.
8. **[Proposto — applicato su delega]** Ogni percorso che arriva da CLI, MCP o da un peer è validato e confinato nella radice del vault prima di toccare il disco.
9. **[Proposto — applicato su delega]** Nessuna perdita silenziosa di dati: ciò che il sync rimuove finisce nel cestino del vault; una divergenza diventa una copia di conflitto; un file illeggibile è «bloccato», mai «assente»; creare una nota non sostituisce mai un file esistente.
10. **[Proposto — applicato su delega]** Lo stdout del server MCP porta solo messaggi di protocollo; i log vanno su stderr.
11. **[Proposto — applicato su delega]** Standard di codice del CLAUDE.md §1, §5, §9: `cargo fmt`, `cargo clippy -D warnings`, doc-comment sulle API pubbliche, niente `unsafe`, niente `unwrap()`/`expect()` fuori dai test, funzioni entro 30 righe e 4 parametri, Conventional Commits.

### V.4 Nucleo di dominio

Per §11.5: se un task tocca questi moduli, l'orchestratore legge anche il diff.

- **[Proposto — applicato su delega]** In `crates/sb-core/src/`: `path.rs` (validazione dei percorsi), `vault.rs` (operazioni sul disco, confinamento, cestino), `index.rs` (risoluzione dei link), `graph.rs`, `ops.rs` (confine con CLI, MCP e GUI).
- **[Proposto — applicato su delega]** In `crates/sb-sync/src/` (da M2): documento di sync e regola «viva/lapide», risoluzione delle rivendicazioni di percorso, riconciliazione disco ↔ documento, protocollo e autenticazione tra peer. File (piano M2): `doc.rs` (documento, lapidi, rivendicazioni di percorso), `plan.rs` (pianificatore della riconciliazione), `local.rs` (esecuzione su disco), `store.rs` (persistenza di documento e base), `auth.rs`, `protocol.rs`, `session.rs`, `replica.rs`.

### V.5 Decisioni

Tutte **[Proposto — applicato su delega]**; il dettaglio e il perché sono nella spec.

- Nome prodotto «Second Brain». Due binari: `secondbrain` (GUI) e `sb` (console: CLI, server MCP stdio, sync headless).
- Workspace Cargo con quattro crate: `sb-core`, `sb-sync`, `sb-cli`, `sb-app`.
- Stack: GUI `eframe`/`egui` 0.37 con renderer OpenGL; anteprima `egui_commonmark` 0.26; parser `pulldown-cmark` 0.13; documento di sync `loro` 1.17 (CRDT, fusione per righe); trasporto `iroh` 1.4 con scoperta in LAN via `iroh-mdns-address-lookup`; osservazione cartella `notify` 8.2; server MCP `rmcp` 3.10; CLI `clap` 4.
- Identità delle note nel sync: id casuale per nota, il percorso è una proprietà. Due note sullo stesso percorso si separano con una regola deterministica (duplicato eliminato oppure copia di conflitto).
- Fiducia: gruppo di sync = conoscenza di un segreto a 32 byte consegnato in un ticket; prova reciproca a ogni connessione.
- Stato di sync fuori dal vault, in una cartella dati per utente; nel vault resta solo `.secondbrain/trash/`.
- Modalità di rete: `internet` (predefinita: DNS e relay pubblici del progetto iroh, che vedono id e indirizzi ma non il contenuto) e `lan` (nessun contatto esterno).
- Milestone: M1 core + CLI + MCP; M2 sync headless; M3 GUI; M4 Linux, prova Windows↔Linux, documentazione.

### V.6 Esclusioni dal grafo graphify

- `target/`, `.git/`, `.superpowers/`, `.claude/`, `.worktrees/`, `graphify-out/`, `dist/`, vault di prova e fixture binarie.

### V.7 Stato e avanzamento

- 2026-11-15 — Repo git inizializzato (branch `main`, identità locale al repo). Toolchain Rust 1.99 MSVC verificata. Docker Desktop avviato: engine Linux disponibile.
- 2026-11-15 — Conclusi i 4 spike API. Scritta la spec, passata al builder check, aggiornata con le rulings. Scritto il piano M1.
- 2026-11-15 — **M1 in corso** nel worktree `.claude/worktrees/m1-core-cli-mcp`, branch `feat/m1-core-cli-mcp`. Avanzamento per task nel ledger `.superpowers/sdd/2026-11-15-m1-core-cli-mcp/progress.md` del worktree.
- 2026-11-16 — M1: task 1–7 completati e revisionati (percorsi, vault, Markdown, indice, grafo e ricerca, operazioni, CLI `sb`). Task 8 (server MCP) in lavorazione. Suite: 97 test su Windows al Task 7; 90 passati nel container Linux al Task 6 (`rust:1-bullseye`). Restano per chiudere M1: review del Task 8, ondata finale di piccole correzioni annotate nel ledger, validazione dell'orchestratore, revisione finale `opus`, `graphify --update`, merge in `main`.
- 2026-11-16 — Bozza del piano M2 scritta: `docs/superpowers/plans/2026-11-16-m2-sync.md` (11 task). **[Aperto]**: va riletta contro il codice reale di M1 prima di eseguirla (in particolare `Vault::adopt_folders`, `IgnoreReason::Unreadable`, la firma di `Replica::open`).
- 2026-11-16 — **M1 completata** (core, CLI `sb`, server MCP `sb mcp`), branch `feat/m1-core-cli-mcp`, testa `c272876` più i commit di documentazione; unita a `main` in locale, nessun push.
  - Task 8 (server MCP, 10 strumenti) revisionato. Revisione finale dell'intero branch su `opus` (sola lettura): «pronto al merge con correzioni» — 1 critico, 2 importanti, 10 minori.
  - Difetti corretti nell'unica ondata finale (9 commit, ri-revisione pulita): `rename` poteva sovrascrivere una nota esistente indicata con il nome corto 8.3 di Windows (riprodotto, ora rifiutato); una sottocartella non elencabile faceva fallire l'intera scansione (ora è «ignorata» e la scansione prosegue); chiamate MCP parallele potevano perdere un `append` (ora le operazioni del server sono serializzate); backslash nel titolo di `[[Nota#Titolo\|Alias]]`; fine riga CRLF in `vault.rs` (aggiunto `.gitattributes`); `read` rifiuta i file oltre 4 MiB; `write` non sostituisce un link simbolico e conserva i permessi; filtro `--folder` validato; descrizioni e annotazioni degli strumenti MCP.
  - Validazione dell'orchestratore: su Windows `cargo fmt --check`, `cargo clippy -D warnings` e 125 test puliti; nel container Linux `rust:1-bullseye` 129 test passati, e i test su permessi e link simbolici passano anche da utente non privilegiato; ogni comando della CLI provato a mano con e senza `--json`; rifiuto di `..`, lettera di unità, `.secondbrain/`, backslash, nomi riservati; `sb mcp` provato via pipe con `initialize`, `tools/list` e una chiamata per strumento: lo stdout contiene solo messaggi di protocollo.
  - Non verificato: il server MCP con i client reali Claude Code e Claude Desktop (provato solo con richieste JSON-RPC scritte a mano e con il client di test di `rmcp`); il comportamento dei nomi riservati su Windows 10 (provato solo Windows 11).
  - Rinviato, annotato nel ledger di M1: a M2 la regola più ampia sui nomi riservati (`CON .md`, `COM0`, `CONIN$`), la creazione di file a prova di crash, il rifiuto uniforme di sovrascrivere una nota in sola lettura; a M3 `ops::create_note`, la regola per gli allegati (`![[image.png]]` oggi è un nodo «mancante» nel grafo), il costo dei backlink a ogni cambio di nota.
- 2026-11-16 — M1 unita a `main` con un commit di merge (`3b49441`); l'albero di `main` è identico a quello del branch. Il worktree `.claude/worktrees/m1-core-cli-mcp` resta sul disco; il ledger di M1 è copiato anche in `.superpowers/sdd/2026-11-15-m1-core-cli-mcp/` nella radice (ignorato da git).
- 2026-11-16 — **M2 avviata.** Piano riletto e corretto: nuovo Task 0 (regola più ampia sui nomi riservati, creazione dei file a prova di crash, nessuna sostituzione di una nota in sola lettura); il pianificatore tratta come «bloccati» i file illeggibili e le cartelle non elencabili; i file delle note remote nascono con la grafia delle cartelle esistenti. Worktree `.claude/worktrees/m2-sync`, branch `feat/m2-sync`; ledger in `.superpowers/sdd/2026-11-16-m2-sync/progress.md` del worktree.
- Rischio aperto, non verificato: sulla macchina di sviluppo il preset di rete predefinito di iroh non è mai risultato «online» (le sonde HTTPS verso i relay falliscono con «network unreachable»). Causa non accertata. Da riprovare in M2; se resta così, la modalità `internet` va dichiarata «non verificata su questa macchina».
- Non verificabile in questo progetto: uso interattivo della GUI da parte di una persona. La GUI si verifica con test headless e con l'avvio del binario.

### V.8 Rulings dell'orchestratore

Formato: decisione — perché — costo se sbagliata.

- 2026-11-15 — **Sessione principale non Opus.** La sessione gira su Fable 5.2. §11.2 dice di non pianificare e avvisare l'utente. Avviso dato in chat; si procede perché l'utente ha ordinato autonomia totale senza soste e Fable è il tier sopra Opus. Restano: esecutori Sonnet/Haiku, `model` esplicito in ogni dispatch, revisione finale di milestone a `opus`, nessuna riga di codice, test o config scritta dall'orchestratore. Costo: l'utente può far rifare la pianificazione su Opus; spec e piani sono documenti rivedibili.
- 2026-11-15 — **Approvazione di spec e piani.** §11.4 vuole l'approvazione dell'utente prima di implementare. L'utente ha delegato ogni decisione: spec e piani valgono come approvati su delega dopo il builder check. Costo: rilavorazione delle parti che l'utente non condivide.
- 2026-11-15 — **Merge locali.** A fine milestone il branch viene unito a `main` in locale senza chiedere. Nessun remote, nessun push. Costo: un `git revert` del merge.
- 2026-11-15 — **`.git/info/exclude`.** L'orchestratore vi ha aggiunto `.claude/worktrees/` e `.superpowers/` per poter creare il worktree prima che esista un `.gitignore` (che è config e lo scrive un implementer). È metadato locale di git, non un file del progetto. Costo: nessuno; si toglie con un editor.
- 2026-11-15 — **Builder check, domanda 1 (file ignorati nella riconciliazione).** Terzo stato «bloccato»: un file illeggibile o ignorato non è mai trattato come assente e non viene mai sovrascritto; la creazione di un file non sostituisce mai nulla. — Senza, un file diventato non UTF-8 avrebbe propagato una cancellazione. — Costo: una nota bloccata resta non sincronizzata finché l'utente non sistema il file; è segnalata nello stato.
- 2026-11-15 — **Domanda 2 (testo raddoppiato, peer non pronto).** Il testo raddoppiato non è accettabile. Si passa a id casuali per nota con risoluzione deterministica delle rivendicazioni di percorso; sparisce lo stato «in attesa del primo sync» come condizione di correttezza, resta solo come attesa di 20 secondi che evita di memorizzare due volte le note. — Elimina raddoppi e stalli. — Costo: più logica nel documento di sync; due creazioni concorrenti con contenuto diverso diventano due note (una «conflict») invece di una nota fusa.
- 2026-11-15 — **Domanda 3 (lapidi e rename).** Le lapidi sono un insieme di coppie (id, hash) mai rimosse; una nota morta non rinasce, un file che riappare è una nota nuova; il rename è un'operazione vera (cambio del percorso della nota) riconosciuta dal confronto degli hash. — Chiude i casi di resurrezione e di copie doppie. — Costo: un rename fatto insieme a una modifica nello stesso giro vale come cancella + crea.
- 2026-11-15 — **Domanda 4 (dimensioni e crescita).** Obiettivo dichiarato: fino a circa 10 000 note e 100 MB di testo; nota massima 4 MiB (oltre è ignorata); frame fino a 1 GiB dopo l'autenticazione e 64 byte prima; nessuna compattazione della storia. — Limiti espliciti invece di un blocco silenzioso. — Costo: vault più grandi non sono supportati in v1; la storia cresce sempre e il testo cancellato vi resta.
- 2026-11-15 — **Domanda 5 (sovrascritture concorrenti).** Si accetta la finestra residua di poche system call tra il ricontrollo e la rinomina; le copie di conflitto nascono dalla stessa regola delle rivendicazioni, quindi rifarle dopo un crash non crea doppioni. — Non esiste un compare-and-swap atomico sui file. — Costo: una scrittura esterna che cade esattamente in quella finestra può andare persa.
- 2026-11-15 — **Domanda 6 (uscire dal gruppo).** Aggiunto `sb sync leave` e «Disattiva sync» nella GUI; rientrare in un gruppo non cancella mai note locali, quindi una nota cancellata dal gruppo durante l'assenza ritorna come nota nuova. — Nessuno stato senza via d'uscita. — Costo: possibili note «resuscitate» dopo un rientro.
- 2026-11-15/16 — **Rulings nate durante M1** (dettaglio e costo nel ledger di M1; la spec è già allineata): nome nel cestino mai oltre 255 byte, accorciato da sinistra; un file illeggibile è «ignorato» (`Unreadable`) e non fa fallire la scansione; il nome nel cestino si prenota in modo atomico; una nota non è mai tra i propri backlink; un link che inizia con `/` vale solo come percorso esatto dalla radice e uno con `./` solo relativo alla cartella della nota; una nota nuova adotta la grafia delle cartelle esistenti; `sb --json read` stampa la nota come stringa JSON mentre lo strumento MCP `read_note` restituisce testo grezzo; l'helper di test a 5 parametri resta con un `allow` locale; un giro di correzioni può partire anche a review pulita quando l'orchestratore, leggendo il nucleo di dominio, trova una lacuna rispetto alla spec.
- 2026-11-16 — **Rulings della chiusura di M1** (dettaglio nel ledger di M1 e in `final-review.md`; la spec 5.3 e 5.4 è allineata):
  - `rename` non sostituisce mai nulla: rifiuta qualunque destinazione che il file system trovi, anche sotto un altro nome (alias 8.3); è ammesso solo il cambio di maiuscole della nota stessa. Resta una finestra di poche system call tra il controllo e lo spostamento, accettata come per la sostituzione (spec 8.5). Costo: una scrittura esterna che cade in quella finestra può andare persa.
  - Una sottocartella che non si può elencare è «ignorata» (percorso con `/` finale) e per il sync ogni nota sotto di essa sarà «bloccata», mai «assente». Solo la radice del vault non elencabile fa fallire la scansione. Costo: le note in quella cartella non compaiono e non si sincronizzano finché i permessi non cambiano; è segnalato.
  - Il server MCP esegue un'operazione alla volta. Tra processi diversi (CLI, MCP, poi GUI e motore) resta la finestra già accettata. Costo: una ricerca lenta fa attendere le altre chiamate dello stesso server.
  - `write_note` su un file che la scansione ignora (non UTF-8, troppo grande) lo sostituisce, perché «scrivi» vuol dire sostituire; leggere un file oltre 4 MiB è rifiutato. Costo: un agente può sovrascrivere un `.md` non testuale su cui gli è stato chiesto di scrivere.
  - Il giro di correzioni del Task 8 è confluito nell'ondata finale invece di girare da solo. Costo: una ri-revisione unica su un diff più grande.
  - graphify gira sulla radice del repository dopo il merge locale, non nel worktree prima: il grafo vive nella radice e non esiste ancora. Costo: due passi di §11.4 invertiti.
  - Le voci che la revisione finale ha giudicato «da fare dopo» o «accettate» sono state adottate come scritte; l'elenco è nel ledger.
- 2026-11-16 — **Parallelismo.** Il revisore (sola lettura, lavora su un file di diff) gira in parallelo all'implementer del task successivo; due implementer non scrivono mai insieme: un giro di correzioni aspetta il commit dell'implementer in corso. Costo: nessuno osservato; se due scritture si sovrapponessero, un conflitto di indice git da ripetere.
- 2026-11-15 — **Domanda 7 (chi fa le prove a mano, connettività in M4).** Le prove «a mano» le esegue l'orchestratore con comandi; la GUI non è verificata da una persona e lo si dichiara; la prova Windows↔Linux usa un ticket con indirizzi e porta fissa; `internet` resta la modalità predefinita e il suo stato reale si registra in M2. — Docker è disponibile; mDNS non attraversa la rete di Docker. — Costo: se `internet` non funziona sulle reti dell'utente, il sync tra reti diverse non va finché non si indaga.
