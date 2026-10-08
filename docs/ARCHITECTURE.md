# Architecture decisions

## Independence

The browser engine is implemented in this repository. Complete external browser engines are not embedded.

## Memory

Memory accounting is a first-class subsystem. Budgets are explicit, resource reservations are RAII-managed, and the resource cache now uses bounded LRU-style eviction and stable key replacement. Later subsystems will report reclaimable caches and memory pressure.

## Networking

Network requests pass through policy before transport. The policy layer carries resource kind, referrer and first-party context so native ad/tracker blocking can distinguish first-party from third-party requests. The current HTTP transport is bounded by response/header memory limits and rejects HTTPS until a native TLS layer exists. Local file URLs are handled separately by a bounded LocalFileTransport, with empty/localhost authorities accepted and remote file authorities rejected. Document loading marks top-level requests as Document resources, supports bounded redirects and persists scoped cookies between redirects. Linked stylesheets are fetched as Stylesheet resources with document referrer/first-party context, CSS Accept/Referer headers and bounded redirects; failed stylesheet resources do not fail the document load. Cookie handling validates Domain scope, enforces Secure delivery and supports Max-Age deletion.

## Navigation

Navigation state is kept separate from transport and document parsing in core::navigation. A NavigationHistory owns ordered NavigationEntry values, tracks the current entry, discards forward entries when a new navigation is pushed, and exposes bounded back/forward traversal. The public Browser API delegates navigation, current URL lookup, back, forward and current-title updates to this state machine. Each tab also retains its active loaded Page so a successful load can be rendered again without coupling transport to the platform surface. TabManager owns independent histories and the active-tab invariant, while Browser exposes tab creation, selection and guarded closing. DOM anchors expose their authored href through a small link-target API; Browser link activation validates the element, trims the destination and resolves it against the active document URL before creating a history entry. Loaded Pages retain the final retrieval URL and can resolve references through a valid HTML <base href> before falling back to that retrieval URL. target="_blank" opens a fresh tab before navigation. Browser reload uses the current URL and replaces the active history entry instead of duplicating it. Form submission now constructs an initial application/x-www-form-urlencoded entry list for input, textarea and select controls, supports GET/POST and can convert the result into a network Request. Non-hierarchical references such as javascript: and mailto: are rejected by URL resolution rather than being treated as relative paths.

## Web engine

HTML tokenization, tree construction, DOM, CSS, layout and rendering are separate layers. The HTML parser currently handles void elements, basic implied document structure, paragraph/list auto-closing, DOM comments and implicit html/head/body construction. The DOM supports recursive element lookup and explicit extraction of anchor href targets; Page retains its retrieval URL, resolves document references through <base>, and extracts the document title without coupling navigation state to rendering. Layout currently supports block/inline flow, HTML-aware default display mapping, intrinsic inline sizing, box-model accumulation, an initial border-box interpretation, percentage widths, initial min/max dimension constraints, fixed-width block auto-margin distribution and explicit pixel line-height for inline flow. CSS border shorthand and standard border-width keywords feed the layout box model. The renderer builds a platform-neutral display list, clears the target viewport before painting, fills backgrounds through the padding box, honors explicit solid borders, suppresses visibility:hidden subtrees, and rasterizes a small bootstrap bitmap glyph set. Page construction extracts inline <style> elements into the stylesheet cascade, while DocumentLoader resolves and loads linked text/css stylesheets before constructing the Page. CSS declaration parsing preserves hash-valued tokens so hexadecimal colors survive into rendering. HTTP response parsing is method-aware for HEAD and bodyless status codes, and rejects conflicting Transfer-Encoding and Content-Length framing. CSS resolution includes inheritance, CSS-wide keywords and initial box shorthand expansion. No renderer-specific assumptions belong in the HTML parser.

## Platform

Windows, Linux and ArchiaOS are represented through a platform abstraction. Web-engine code should not call operating-system APIs directly.

## Validation

GitHub Actions is the authoritative validation environment. Local compilation is not used as project validation.
