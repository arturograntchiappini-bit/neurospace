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

Ultimo aggiornamento: 2026-11-18 (chiusura di M3)

**Legenda stati.** **[Confermato]** = detto o approvato dall'utente. **[Proposto — applicato su delega]** = deciso dall'orchestratore in base alla delega totale del 2026-11-15, non approvato voce per voce dall'utente: vale finché l'utente non lo cambia. **[Aperto]** = non deciso. Ciò che non è verificato è scritto come «non verificato».

### V.1 Visione

- **[Confermato]** (richiesta dell'utente, 2026-11-15, testo integrale): «crea un'applicazione per prendere appunti, come obsidian, su base markdown e grafi, sincronizzazione p2p serverless tra dispositivi (anche tra os diversi come windows e linux (quasi ogni distro), ma niente mac), come ui per ora falla simile a obsidian non troppo complicata. crea l'applicazione in full rust, e con una porta di accesso nativa per agenti ai, mcp o cli o connettori custom per claude. usa i workflow previsti dal claude.md. procedi in totale completa autonomia fino a completamento, ti ho detto tutto quello che avevo da dirti. prendi ogni decisione in autonomia seguendo le istruzioni date.»
- **[Confermato]** Delega: l'utente ha delegato ogni decisione e chiesto di procedere senza fermarsi fino al completamento (2026-11-15).
- **[Confermato]** (richiesta dell'utente, 2026-11-17, testo integrale, arrivata all'inizio della pianificazione di M3): «la UI deve essere molto semplice e minimal ma molto molto ben leggibile e accessibile, polished, un po' stile mac con effetto glass».
- **[Confermato]** (richiesta dell'utente, 2026-11-17, dopo aver visto le schermate dello spike sul vetro, testo integrale): «i saw the preview of spike glass i kinda like it, but i don't like the purple/pink gradient, get rid of it, plus, the glass effect should be refined better». Quindi: niente sfondi colorati a gradiente, solo fondali neutri; il vetro va rifinito.
- **[Confermato]** (richiesta dell'utente, 2026-11-18, testo integrale): «as logo and icon use assets/logo/second-brain.svg». L'SVG è la sola fonte del logo e dell'icona (spec 9.6, «Logo and icon»).

### V.2 Fonti

- Richiesta dell'utente (V.1).
- **[Proposto — applicato su delega]** Spec di design: `docs/superpowers/specs/2026-11-15-second-brain-design.md`. Letta da un revisore indipendente (builder check) il 2026-11-15; le sue 7 domande sono state risolte come rulings (V.8) e la spec aggiornata.
- **[Proposto — applicato su delega]** Piano M1: `docs/superpowers/plans/2026-11-15-m1-core-cli-mcp.md`. Piano M2: `docs/superpowers/plans/2026-11-16-m2-sync.md` (12 task, da 0 a 11), riletto contro il codice di M1 il 2026-11-16. Piano M3: `docs/superpowers/plans/2026-11-17-m3-gui.md` (10 task), scritto il 2026-11-17 dopo lo spike sul vetro, eseguito e chiuso il 2026-11-18 (più il logo, chiesto durante la chiusura). Il piano M3b (prestazioni del motore di sync e i cinque minori di stato rinviati da M2) e quello di M4 si scrivono all'inizio di ciascuno — **[Aperto]**.
- Spike sul vetro del 2026-11-17 (codice usa-e-getta nella scratchpad): materiale Mica di Windows tramite `window-vibrancy` 0.8.1, vetro simulato, font Inter 4.1, contrasto WCAG. I fatti verificati sono nell'appendice A.5 della spec, e la sezione 9.6 contiene l'aspetto scelto.
- Spike di verifica API del 2026-11-15 (codice usa-e-getta, fuori dal repo): iroh 1.4, eframe 0.37 + egui_commonmark 0.26, rmcp 3.10, loro 1.17 + notify 8.2. Conclusi; i fatti verificati sono nell'appendice A della spec, con accanto ciò che non è stato verificato.

### V.3 Vincoli non negoziabili

Da copiare in ogni dispatch a un implementer (§11.4).

1. **[Confermato]** Tutto in Rust: nessun frontend web/JS, nessun altro linguaggio nel prodotto.
2. **[Confermato]** Le note sono file Markdown; il prodotto offre una vista a grafo dei collegamenti.
3. **[Confermato]** Sincronizzazione P2P serverless tra dispositivi, funzionante tra Windows e Linux (quasi ogni distro). macOS escluso.
4. **[Confermato]** UI simile a Obsidian, non troppo complicata. Dal 2026-11-17: molto semplice e minimale, molto ben leggibile e accessibile, curata, un po' in stile macOS con effetto vetro (glass).
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
- Milestone: M1 core + CLI + MCP; M2 sync headless; M3 GUI; M3b prestazioni del motore di sync e minori di stato (dal 2026-11-17); M4 Linux, prova Windows↔Linux, documentazione.

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
- 2026-11-16 — Grafo di conoscenza costruito per la prima volta sulla radice (`graphify-out/`, ignorato da git) da un subagente `haiku`: 728 nodi, 1612 archi, 25 comunità; 27 file letti (21 di codice, 6 documenti), nessun nodo da `.claude/`, `target/` o `.superpowers/`. Le esclusioni di V.6 sono nel file `.graphifyignore`. Da sapere: `CLAUDE.md` e il piano M2 sono cambiati durante la costruzione, quindi nel grafo c'è la loro versione precedente; si riallineano al prossimo `graphify --update` (fine M2).
- 2026-11-16 — M2, avanzamento: completati e revisionati i task 0–6 (irrobustimento del core; cartella di stato, ticket e prova di autenticazione; documento di sync su `loro`; persistenza di documento e base; pianificatore della riconciliazione; esecuzione su disco; protocollo e sessioni autenticate). Task 7 (repliche che si scambiano aggiornamenti, con la suite di scenari) in lavorazione. Restano i task 8–11 (ingresso e uscita dal gruppo, motore e osservazione della cartella, rete `iroh`, comandi `sb sync`). All'ultimo controllo (commit `3a85576`): 283 test passati nel container Linux, 19 test sensibili ai permessi passati anche da utente non privilegiato; su Windows suite verde. Non ancora fatto: `clippy` su Linux (l'immagine non lo contiene; va installato nel container alla validazione di milestone).
- 2026-11-16 — M2, avanzamento: completati e revisionati anche i task 7–9 (repliche che si scambiano aggiornamenti, con 26 scenari; ingresso e uscita dal gruppo, comandi via file `command.pending`; osservazione della cartella e motore a thread unico). Task 10 (rete `iroh`) in lavorazione, Task 11 (comandi `sb sync`) da fare. All'ultimo controllo (commit `549800a`): 361 test passati nel container Linux, 20 test sensibili ai permessi passati da utente non privilegiato; su Windows suite verde.
  - Difetto trovato dalla revisione del Task 9 e corretto: su Linux l'osservatore della cartella contava anche le semplici letture come modifiche, quindi ogni scansione ne provocava un'altra (circa 3 al secondo, per sempre). Su Windows non si vedeva perché Windows non segnala le letture.
  - Dalla sera del 2026-11-16 la sessione principale gira su Opus 5.5 (scelto dall'utente con `/model opus`), come chiede §11.2; il ruling «sessione non Opus» vale solo per il lavoro precedente.
- 2026-11-17 — **M2 completata e validata** (sync P2P headless, comandi `sb sync run|invite|join|leave|status`), branch `feat/m2-sync`, testa `ba804ad`.
  - Revisione finale `opus` (sola lettura): «pronto al merge con correzioni» — 0 critici, 3 importanti, 7 minori, più gli 11 punti già noti. Un'unica ondata finale di correzioni (6 commit, gruppi A–F) li ha chiusi; la ri-revisione ha trovato 8 minori, giudicati come in V.8 (2 documentali corretti subito, 1 accettato, 5 rinviati a M3).
  - Corretto, tra l'altro: un vault che sparisce durante la scansione (chiavetta tolta, disco smontato) veniva letto come «tutte le note cancellate» e la cancellazione arrivava a tutto il gruppo; senza la cartella `.secondbrain` o con uno stato danneggiato non c'era modo di uscire (`sb sync leave` ora funziona sempre e il messaggio d'errore dice cosa fare); un messaggio vuoto da un peer costava due passate complete sul vault.
  - Validazione dell'orchestratore: su Windows `fmt`, `clippy -D warnings` e 424 test puliti (2 ignorati); nel container Linux `rust:1-bullseye` (rustc 1.98) `clippy` pulito, 436 test passati e i test sensibili ai permessi passati da utente non privilegiato; prova a due processi reali in modalità `lan`: ogni modifica (scrittura, aggiunta, modifica a mano, cancellazione con il file nel cestino) converge in 0,4 s, nessun segreto nei log; Ctrl+C di `sb sync run` su Windows verificato (esce con codice 0 e libera il lock).
  - Misura alla scala dichiarata (10 000 note, 103 MB, Windows, `lan`): prima scansione 114 s, copia completa verso un dispositivo vuoto 28 s, una modifica arriva all'altro dispositivo in circa 4,2 s, CPU a riposo trascurabile, stato di sync 108 MB per dispositivo. Il README dice «progettato per», non «dimensionato per». Migliorare la latenza (scansione incrementale) è un compito di M3.
  - Modalità `internet`: nessun difetto nel codice. Su questa macchina un firewall applicativo (simplewall) blocca le connessioni in uscita degli eseguibili che non conosce: una copia identica di `curl.exe` viene bloccata, l'originale no. Per questo `sb` non raggiungeva mai un relay. Nel container Linux sulla stessa macchina due motori senza indirizzi nel ticket si trovano tramite la scoperta pubblica di iroh in 1,4–6 s. `sb sync status` ora mostra il relay raggiunto e, se manca, suggerisce di controllare il firewall.
  - Non verificato: la modalità `internet` tra due reti reali diverse e su Windows (bloccata qui dal firewall); la scoperta mDNS tra due macchine; che errore dà Windows quando il vault viene smontato durante una scansione; mDNS aggiunto all'endpoint già avviato su una rete reale (controllato solo nel sorgente di iroh).
- 2026-11-17 — `graphify --update` dopo M2 (subagente `haiku`; estrazione semantica dei 5 documenti cambiati in tre blocchi `haiku` lanciati dalla sessione principale): da 728 nodi, 1612 archi e 25 comunità a 2083 nodi, 5651 archi e 120 comunità. La guardia di restringimento non è scattata e nulla è stato forzato. Scompaiono 3 nodi, tutti attesi: `WINDOWS_DEVICE_NAMES`, tolto da `path.rs` nel Task 0 di M2, e due segnaposto della libreria standard. Nessun nodo dalle cartelle escluse (V.6). Non fatto: `cost.json` e la cache semantica dei documenti, quindi il prossimo aggiornamento ri-estrae quei documenti. graphifyy installato 0.9.93, skill 0.9.105: non aggiornato.
- 2026-11-17 — **Pianificazione di M3.** Spike sul vetro in due versioni: la v1 aveva uno sfondo a gradiente viola/rosa, rifiutato dall'utente; la v2 usa fondali neutri e solo le aree strumenti sono di vetro, con la nota su una superficie piena. Spec aggiornata: 6.1 (link agli allegati), 6.3 (fantasmi dei link ancorati), 6.6 (dove nasce una nota creata da un link), 9 (vetro, tema di sistema, accessibilità, impostazioni, ciclo di vita del motore nella GUI), A.5 (fatti dello spike). Scritto il piano M3 (10 task).
- 2026-11-18 — **M3 completata e validata** (GUI `secondbrain`): branch `feat/m3-gui`, worktree `.claude/worktrees/m3-gui`, testa `7783575` più i commit di documentazione. Ledger con ogni ruling: `.superpowers/sdd/2026-11-17-m3-gui/progress.md` del worktree.
  - Dieci task completati e revisionati: albero, editor, anteprima, grafo, ricerca, apertura rapida, impostazioni, sync dalla GUI, vetro Mica o simulato, tema di sistema, accessibilità.
  - Revisione finale `opus` (sola lettura): «pronto al merge con correzioni», 0 critici, 2 importanti, 7 minori. I due importanti:
    - un salvataggio fallito intrappolava l'utente e la seconda chiusura perdeva il testo;
    - a 10 000 note ogni salvataggio automatico bloccava l'interfaccia per 80–125 ms.
  - Un'unica ondata di correzioni (5 commit). La ri-revisione ha trovato 2 importanti e 6 minori. Corretti:
    - la GUI poteva creare un gruppo nuovo e usare la rete dopo un `sb sync leave` fatto altrove;
    - il file `recovered/` non veniva mai scritto alla chiusura.
    - Gli altri sono accettati o rinviati a M3b (V.8).
  - Dalla validazione:
    - «Adatta grafo» ingrandisce fino al 200 %, così i nomi delle note si vedono;
    - logo e icona dall'SVG dell'utente (finestra, barra delle applicazioni, icona dell'eseguibile, schermata di benvenuto, README).
  - Validazione dell'orchestratore:
    - Windows: `fmt`, `clippy -D warnings` e tutta la suite puliti (sb-app 170 unitari più le suite UI, sb-core 125, sb-sync 218).
    - Container Linux `rust:1-bullseye` (rustc 1.98), su `7783575`: `fmt` e `clippy` puliti, ogni suite passata da root (sb-app 171, sb-core 129, sb-sync 225), 11 test sensibili ai permessi passati da `nobody`. I 7 test dei salvataggi falliti sono passati da `nobody` su `dffbe03`.
    - Binario di release, vault di 3 note, catture della sola finestra dell'app: scuro e chiaro, con Mica e con «Riduci trasparenza». Vetro neutro, nessun gradiente, nota su superficie piena, testo leggibile, anello di focus visibile.
    - Un vault mai sincronizzato non avvia il motore: la cartella di stato contiene solo `vault-path.txt`. Ogni chiusura esce con stderr vuoto.
    - Vault di 10 000 note (94,7 MB):
      - albero disegnato in 1,0–1,6 s;
      - ricerca 45–116 ms per tasto (381 ms per una sola lettera);
      - una modifica esterna della nota aperta compare in 1,9 s;
      - memoria 228–243 MB.
  - Non verificato:
    - uso interattivo da parte di una persona (trascinamento, sensazione di Mica);
    - Windows 10;
    - la GUI su Linux con un display reale: solo build e test headless;
    - l'icona sul pulsante della barra delle applicazioni: si cattura solo la finestra;
    - un bordo nero sottile nelle catture opache, probabilmente il bordo di ridimensionamento reso da PrintWindow.
    - Visto una volta e non riprodotto: con la finestra ridotta a icona il processo è salito a 1 GB per poi scendere a 533 MB; tre prove successive sono rimaste a circa 228 MB.
- Rischio chiuso il 2026-11-17: il preset di rete di iroh che «non risultava mai online» su questa macchina era il firewall applicativo, non il codice né iroh.
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
- 2026-11-16 — **Rulings nate durante M2** (dettaglio e costo nel ledger di M2; spec 5.2, 5.4 e 8.3 allineate):
  - La base non sopravvive mai al documento su cui è stata salvata: il motore registra in `base.version` la versione del documento e, all'avvio, se il documento non la contiene più (log danneggiato o cancellato, cartella di stato ripristinata a metà), svuota la base. Senza questa regola ogni nota sarebbe letta come «cancellata altrove» e spostata nel cestino. Costo: dopo uno svuotamento, le note che differiscono tra disco e documento diventano copie di conflitto.
  - Il controllo che il vault sia davvero presente (cartella `.secondbrain`) si fa all'inizio di ogni giro, non solo all'avvio: un disco smontato che lascia una cartella vuota non deve far cancellare le note a tutto il gruppo. Costo: resta una finestra tra la scansione e il salvataggio del documento.
  - I file di stato (segreto del gruppo, chiave del dispositivo, documento) sono creati leggibili solo dall'utente su Linux (0600, cartelle 0700). Il segreto non compare mai in messaggi d'errore né nell'output di debug.
  - Regola più ampia sui nomi riservati di Windows, creazione dei file a prova di crash (file temporaneo, poi collegamento fisico; su FAT/exFAT creazione diretta), nessuna sostituzione di una nota in sola lettura su nessun sistema.
  - Il registro degli aggiornamenti si ripara da solo dopo una scrittura fallita (si riparte sempre dalla fine dell'ultimo blocco valido).
  - Una sessione annullata dall'host segnala comunque la propria chiusura; un dispositivo non si sincronizza con sé stesso.
  - I file temporanei `.sb-*.tmp` rimasti dopo un crash non vengono mai ripuliti (nascosti e innocui); la rinomina «senza sostituzione» resta controllo-poi-rinomina, con la finestra già accettata.
  - Dopo un cambio di gruppo il motore scarta ogni evento di una connessione autenticata con il segreto precedente: ogni evento porta un'impronta di 8 byte del segreto con cui la sessione si è autenticata (non rivela il segreto). Senza, una connessione del vecchio gruppo ancora in coda poteva versare la storia del vecchio gruppo, testo cancellato compreso, nel documento appena svuotato del nuovo. Costo: un campo in più su ogni evento di sessione.
  - `sb sync invite` crea chiave del dispositivo e segreto se mancano e stampa il ticket, come dice la spec 8.8; il piano diceva di rifiutare finché il motore non era mai partito. Costo: una piccola funzione condivisa con la rete.
  - L'osservatore della cartella ignora gli eventi di sola lettura. Costo: nessuno noto; una scrittura arriva sempre anche come evento di modifica.
  - Alcune ri-revisioni di correzioni piccole sono andate a `haiku` o sono state sostituite dalla lettura del diff da parte dell'orchestratore quando il cambiamento era di soli test o di poche righe: costo, un difetto sottile arriva alla revisione finale.
- 2026-11-16 — **Parallelismo.** Il revisore (sola lettura, lavora su un file di diff) gira in parallelo all'implementer del task successivo; due implementer non scrivono mai insieme: un giro di correzioni aspetta il commit dell'implementer in corso. Costo: nessuno osservato; se due scritture si sovrapponessero, un conflitto di indice git da ripetere.
- 2026-11-17 — **Rulings della chiusura di M2** (dettaglio e costo nel ledger di M2; la spec 8.3 è allineata):
  - `config.json` si legge una volta, all'avvio del motore: cambiare modalità, porta o mDNS richiede di riavviare `sb sync run` (spec 8.3 corretta; il README lo dice e consiglia una `bind_port` fissa su ogni dispositivo in modalità `lan` senza mDNS). Costo: una modifica alla configurazione aspetta il riavvio.
  - `rust-version = 1.98`, la toolchain più vecchia davvero provata (iroh dichiara 1.91, ma le API della libreria standard usate non sono state controllate). Costo: chi ha 1.91–1.97 viene rifiutato anche se forse compilerebbe.
  - Un `base.json` illeggibile vale come base vuota (come la guardia della spec 8.3: una base vuota non può spostare nulla nel cestino) ed è segnalato. Costo: le note diverse tra disco e documento diventano copie di conflitto.
  - `sb sync leave` senza motore attivo prende il lock e cancella i file del gruppo senza aprire documento né vault. Costo: nessuno noto.
  - Un nome che differisce da un'altra nota solo per maiuscole (possibile su Linux) blocca la chiave: nessuna delle due varianti vale come modifica o cancellazione. Costo: quella nota non si sincronizza finché non si rinomina una variante; è segnalata come bloccata.
  - Modalità `internet`: nessuna correzione del codice; si aggiunge il relay allo stato con un suggerimento sul firewall e un test (ignorato di norma) che collega due motori per solo id. Costo: un campo in più nello stato.
  - Nessun accorpamento degli aggiornamenti in coda: la latenza di circa 4 s per modifica a 10 000 note viene dalle passate complete sul vault, non dalle raffiche; scansione incrementale e vista del documento in cache vanno a M3 (obiettivo: una modifica sotto 1 s), insieme allo stato scritto anche durante i giri lunghi. Costo: fino ad allora, alla scala dichiarata, una modifica impiega circa 4 s e il primo giro circa 2 minuti.
  - Ri-revisione dell'ondata finale: la lunghezza di due funzioni è accettata perché `clippy` (`too_many_lines` a 30) la misura e passa; CHANGELOG e README corretti dove promettevano cose che il codice non fa; rinviati a M3, perché la GUI rifà lo stato, cinque minori senza perdita di dati (avviso «osservazione della cartella spenta» che sparisce dallo stato, avvisi che restano nello stato fino al riavvio, sessioni vecchie dopo un segreto cambiato a mano, `sb sync status` che legge da sé `config.json`, invitante non registrato in silenzio con `peers.json` pieno).
- 2026-11-17 — **Rulings della pianificazione di M3** (spec 6.1, 6.3, 6.6, 9 e A.5 allineate):
  - Il lavoro sulle prestazioni del motore (scansione incrementale, vista del documento in cache, stato scritto durante i giri lunghi) e i cinque minori di stato rinviati da M2 passano dal piano M3 a un piano M3b, da eseguire dopo M3 e prima di M4: cambiano il motore, non la GUI, e un piano per sottosistema resta rivedibile. Costo: per tutta M3 la latenza alla scala dichiarata resta circa 4 s.
  - Vetro: Mica su Windows quando il sistema lo accetta; vetro simulato su fondale neutro su Linux e dove Mica viene rifiutato; «Riduci trasparenza» rende tutto opaco. Acrylic è scartato perché ignora chiaro/scuro e ingrigiva il tema chiaro. Costo: Mica non è stato provato con il trascinamento sentito da una persona né su Windows 10.
  - I colori sono fissati da una tabella (spec 9.6) e un test controlla il contrasto WCAG ≥ 4,5:1 nel caso peggiore. Con Mica il vetro sale al 76 % (chiaro) e all'82 % (scuro), perché sotto c'è un fondale sconosciuto. Costo: con Mica si vede meno trasparenza di quanta ne mostrava lo spike.
  - La GUI avvia il motore solo se il vault ha già un gruppo, oppure quando l'utente crea un invito o si unisce: un vault mai sincronizzato non contatta la rete. Costo: nessuno noto.
  - L'indice della GUI si ricostruisce per intero in background dopo 300 ms di quiete, invece di aggiornarsi un pezzo alla volta: il watcher non dà i percorsi, e così l'elenco dei file ignorati resta giusto. Costo: alla scala dichiarata ogni cambiamento esterno costa una ricostruzione di qualche secondo in background.
  - Una nota creata da un link con nome semplice nasce nella radice del vault (come in Obsidian); un link `./x` nasce nella cartella della nota di partenza. L'id di un fantasma è la chiave della nota che il clic creerebbe. Costo: i fantasmi dei link `./` cambiano id nell'output di `get_graph`.
  - Inter 4.1 è incorporato nel binario, con la licenza OFL accanto ai font.
- 2026-11-18 — **Rulings nate durante M3 e alla sua chiusura** (dettaglio e costo nel ledger di M3, `.superpowers/sdd/2026-11-17-m3-gui/progress.md`):
  - Un salvataggio che fallisce non intrappola l'utente: su ogni uscita dalla nota (cambio nota, cambio vault, chiusura) il testo finisce in una copia di conflitto e si procede; alla seconda chiusura, se nemmeno quella riesce, il testo va in `<cartella di stato>/recovered/`. La spec 9.3 («il testo scritto non si perde mai») prevale sul ruling del Task 7 «la seconda chiusura esce senza salvare». Costo: più copie di conflitto quando una nota è in sola lettura.
  - Cancellare la nota aperta salva prima il testo non salvato; se il salvataggio fallisce, la cancellazione è rifiutata. Costo: nessuno noto.
  - I backlink e la ricerca non si ricalcolano a ogni salvataggio della nota aperta (chiavi di cache che ignorano il proprio salvataggio). Resta un ricalcolo per salvataggio dopo la ricostruzione in background (N1), rinviato a M3b. Costo: fino ad allora, a 10 000 note, un piccolo scatto dopo ogni pausa di scrittura.
  - La GUI riavvia il motore solo se il vault ha ancora un gruppo: dopo un `sb sync leave` fatto altrove non crea un gruppo nuovo e non contatta la rete. Un invito o un'unione falliti che hanno avviato il motore lo fermano. Costo: un invito ripetuto riavvia il motore (qualche secondo).
  - Alla chiusura la GUI aspetta al massimo 10 s un avvio del motore in corso; al cambio di vault il vecchio motore si ferma in background, senza bloccare l'interfaccia. Costo: una chiusura durante un avvio lento può durare fino a 10 s.
  - «Adatta grafo» ingrandisce fino al 200 %, perché i nomi delle note compaiono dal 150 %: prima un vault piccolo mostrava punti senza nome. Costo: nessuno per i vault grandi, che restano molto sotto il 100 %.
  - Logo: PNG e ICO sono generati a mano dall'SVG e salvati accanto, con i comandi in `assets/logo/README.md`; la build non ha un renderer SVG. L'icona di `secondbrain.exe` è una risorsa Windows (`winresource`, attiva solo per i target Windows). Il file `.desktop` per Linux va a M4. Costo: se l'SVG cambia, le immagini vanno rifatte a mano.
  - Dipendenze aggiunte a `sb-app` oltre al piano: `raw-window-handle` 0.6.2 (serve a Mica) e `pulldown-cmark` 0.13 (già nel workspace). Il CHANGELOG resta tutto sotto `[Unreleased]` fino al rilascio di M4.
  - Rinviati a M3b: ricostruzione completa dell'indice dopo ogni salvataggio (m5, N1), nessun taglio degli archi fuori vista nel grafo (m6), albero non virtualizzato, stato del motore letto in modo bloccante, i test del motore che vanno in timeout sotto carico. Accettati: ricerca non aggiornata per il testo della nota aperta fino al prossimo cambio tra note (N3); sotto circa 390 pt di larghezza la larghezza minima del contenuto vince sul limite del pannello (N6); una copia di conflitto ridondante in un caso raro (N7); due rami non testati del controllo del sync (N8); sezioni ravvicinate nella finestra del sync (V2). Costo: CPU e fotogrammi a 10 000 note, nessun dato a rischio.
  - Alcune ri-revisioni di correzioni piccole (Task 4, Task 10, correzioni residue, adattamento del grafo, logo) sono state sostituite dalla lettura del diff da parte dell'orchestratore. Costo: un difetto sottile arriva alle revisioni di M3b.
- 2026-11-15 — **Domanda 7 (chi fa le prove a mano, connettività in M4).** Le prove «a mano» le esegue l'orchestratore con comandi; la GUI non è verificata da una persona e lo si dichiara; la prova Windows↔Linux usa un ticket con indirizzi e porta fissa; `internet` resta la modalità predefinita e il suo stato reale si registra in M2. — Docker è disponibile; mDNS non attraversa la rete di Docker. — Costo: se `internet` non funziona sulle reti dell'utente, il sync tra reti diverse non va finché non si indaga.
