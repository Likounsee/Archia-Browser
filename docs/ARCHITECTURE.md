# Architecture decisions

## Independence

The browser engine is implemented in this repository. Complete external browser engines are not embedded.

## Memory

Memory accounting is a first-class subsystem. Budgets are explicit, resource reservations are RAII-managed, and the resource cache now uses bounded LRU-style eviction and stable key replacement. Later subsystems will report reclaimable caches and memory pressure.

## Networking

Network requests pass through policy before transport. The policy layer carries resource kind, referrer and first-party context so native ad/tracker blocking can distinguish first-party from third-party requests. The current HTTP transport is bounded by response/header memory limits and rejects HTTPS until a native TLS layer exists. Document loading supports bounded redirects and persists scoped cookies between redirects.

## Web engine

HTML tokenization, tree construction, DOM, CSS, layout and rendering are separate layers. The HTML parser currently handles void elements, basic implied document structure, paragraph/list auto-closing and DOM comments. CSS resolution includes inheritance, CSS-wide keywords and initial box shorthand expansion. No renderer-specific assumptions belong in the HTML parser.

## Platform

Windows, Linux and ArchiaOS are represented through a platform abstraction. Web-engine code should not call operating-system APIs directly.

## Validation

GitHub Actions is the authoritative validation environment. Local compilation is not used as project validation.
