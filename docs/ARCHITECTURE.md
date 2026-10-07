# Architecture decisions

## Independence

The browser engine is implemented in this repository. Complete external browser engines are not embedded.

## Memory

Memory accounting is a first-class subsystem. Budgets are explicit, resource reservations are RAII-managed, and later subsystems will report reclaimable caches and memory pressure.

## Networking

Network requests pass through policy before transport. This is the foundation for native ad and tracker blocking, permissions and future security checks.

## Web engine

HTML tokenization, tree construction, DOM, CSS, layout and rendering are separate layers. No renderer-specific assumptions belong in the HTML parser.

## Platform

Windows, Linux and ArchiaOS are represented through a platform abstraction. Web-engine code should not call operating-system APIs directly.

## Validation

GitHub Actions is the authoritative validation environment. Local compilation is not used as project validation.
