# flare-plugin-protocol

Business-neutral contracts shared by platform control, Core capability routing and plugin SDKs. This crate has no service, database, broker, deployment or commercial authorization dependency.

`PluginManifest::from_json` bounds input and validates identifiers, semantic/API versions, operation schemas, permission references, data scopes, dependencies, runtime limits and UI declarations. Unknown fields are rejected. The generated JSON Schema checks structure; semantic validation remains mandatory. Server runtimes require an artifact digest/signature reference; fetching and verifying actual artifact bytes is the runtime's responsibility.

Plugins own their visible names: `displayName` is the default, and optional
`displayNameI18n` maps locale tags to translated names, for example
`{"zh-CN": "投票", "en-US": "Poll"}`. It holds at most 16 nonempty names,
each at most 160 UTF-8 bytes. `localized_display_name` selects the requested
locale case insensitively, then the same language, then `displayName`.
Names are presentation only; routing and authorization use `pluginId`.
An empty translation map is omitted, preserving serialization of existing manifests.

`PublisherTrustStore` verifies detached Ed25519 signatures over exact manifest bytes, binding key id to publisher id. `VerifiedManifest` cannot be constructed by clients without verification. `PluginRegistry` accepts verified manifests, preserves immutable versions, rejects namespace takeovers and resolves version-pinned dependency closures with bounded backtracking. This in-memory domain model requires a platform persistence adapter before online publication.

`TenantPluginGrant` separates approval from tenant enable and platform flags. Authorization requires matching tenant, plugin version and manifest digest, approved operation/permission/data scope, actor permission and a valid projection time window. The receiving adapter must verify the control-plane projection signature and derive actor identity from verified transport. This type is not a substitute for authentication.

A plugin with user operations declares `features`: switches an application publishes separately, with the global key `<pluginId>.<id>`. A feature gates user operations, `user` mode service operations and contributions, which Core closes without touching the plugin process, or one boolean client-visible configuration key, which the plugin enforces. `closed_operations`, `closed_service_operations` and `closed_contributions` answer what a map of feature states closes; a missing key counts as closed. `ProjectedPlugin.features` carries that map and `TenantPluginGrant.suspended` a platform stop, which denies every invocation. `PluginManifest::kind` derives `im` or `business`: an `im.` or `social.` host capability or a message-content contribution makes a plugin `im`. Each contribution may carry a `preview` sample for the marketplace (an object of at most 8 KiB), and `admin.lifecycle.uninstall.check` names the admin `query` the platform calls before uninstalling.

The unit of publication is a package (`crate::package`): a zip with the extension `.fpkg` holding `manifest.json`, `manifest.sig` (`{"keyId", "signature"}`), `assets/` and optional `README.md` and `CHANGELOG.md`. The signature and digest stay those of the exact `manifest.json` bytes, and the manifest's `assets` table holds each asset's path, digest and size, so changing a file fails `PluginPackage::open`. Opening also bounds the package to 32 MiB and refuses unsafe paths, duplicate names, inconsistent zip headers, undeclared files and compressed entries. `write_package` stores sorted entries with fixed metadata, so the same contents give the same bytes.

`PluginLifecycle` models validated start, running, drain, stop and failure with revision fencing. Runtime adapters own effects, health, resource isolation and recovery; lifecycle metadata does not start processes.

`UiDocument` is data only. Besides labelled fields and actions it carries display blocks (text, fields, choices, entries, people, status, progress, time, divider, note, and rows on admin pages and pages) and form inputs (text, textarea, number, toggle, select, datetime, options, members); actions may collect a form, ask for confirmation and carry a style. `UiCatalog::resolve_with_state` resolves a card over its payload, the entity's shared state and the viewer's local state, in that order, into a flat `ResolvedPluginView`; `validate_form_values` checks submitted forms with the same rules on clients and plugin services. Contributions may bind an entity (`entityPath`), allow forwarding, restrict a composer entry to an audience, or declare a detail view; console pages are declared in the `admin` section. Every block and the detail take `hiddenWhen` conditions like actions (at most four, read on the merged card data or, on an admin page, the page): a matching block is left out, and a matching detail resolves to `detail: false`. An entries block reports `more` only while the contribution offers its detail, and a string in its array is the entry text when no `textPath` is given, so a string list kept in shared state renders as is.

Further optional document fields, all absent from older manifests:

- Conditions take `allOf: [condition]` (up to four, one level, no `path` of its own) besides `anyOf`, `viewer` and `exists`, in every `hiddenWhen` and `closedWhen`.
- A choices block declares static `options: [{label}]` instead of `optionsPath`; labels go through `texts`, counts and the selection are still read from their paths.
- An entries block's `userIdsPath` names an array of user ids aligned with the entries by position, for string entries such as solitaire lines.
- The rows block of a page or an admin page lists its filter inputs in `filters`; `validate_inputs` checks only those, and the other inputs belong to row action forms. Without `filters` every input is a filter.
- An options or members input reads its largest item count from `maxPath`, falling back to the declared `max`.

Composer data: a composer-action document is resolved, its form validated (`validate_form_over`) and its action input bound against `{"config": <client-visible configuration>}` (`composer_data`), so `defaultPath` and `maxPath` read `/config/<key>`. No message payload or state layer takes part.

Detail paging: a detail with `cursorPath` names the next-page cursor in the detail result. The resolved detail view carries it as `nextCursor` (a non-blank scalar of at most 256 bytes; anything else means the last page). The first page is requested with the detail input alone. The next page is requested with the same input plus `cursor` (`UiDetail::page_input`, also for an admin row's `detailInput`), resolved against the same document, and appended with `ResolvedPluginView::append_page`: entries blocks pair up in order and number on, everything else stays as on the first page. A plugin returns at most an entries block's `limit` per page.

Plugin platform v2 (all optional, absent from older manifests):

- Operation `mode`: `command` (default) or `query`. Only a query may be called in the background: a card's `viewerState`, an account entry's `summary` and a composer entry's `availability`.
- Viewer layer: a message-content contribution's `viewerState: {operation, input, refreshOn: ["present" | "sharedVersion"], minIntervalMs}` (a declared query, `minIntervalMs` at least 1000) is read by the core SDK while the card is presented; the result is overlaid on the card's local layer with `basedOnVersion`. `PluginMessageState.basedOnVersion` names the shared version the local values were computed from: while it is at least the applied shared version, local keys replace shared keys of the same name; once the shared layer is newer they do not, and the other local keys still apply (`UiStateLayers::merge`).
- Grouped entries: an entries block's `groupPath`, `groupLabelPath`, `groupCountPath` and `groupTotalPath` (read inside each entry) group consecutive entries with the same key. The resolved block has `grouped: true`, the first entry of each group carries `group: {key, label, count?, total?, percent?}`, and `index` counts within the group; `append_page` continues a group across pages. `numbered: false` shows no numbers.
- Menu actions: an action's `placement: "menu"` puts it in the card's overflow menu; the resolved view lists those in `menuActions`, apart from the button row `actions`. Block-owned actions stay in their block.
- Account entries: the `account-entry` kind is a "Me" screen entry; its document holds one action targeting a `page` contribution of the same plugin, `icon` and `order` place it, and an optional `summary: {operation, path}` (a query) gives its subtitle.
- Composer entry availability: a `composer-action` may declare `availability: {operation, path}` (a query taking `{}`) for a capability that depends on the deployment, such as payments that need an identity service. The core SDK asks it before listing the entry and lists it only when the value at `path` is the boolean `true`; an error, a refusal, a timeout or any other value hides it (`UiEntryAvailability::admits`). Presentation only: the plugin still refuses the invocation itself.
- Entity lifecycle: a message-content contribution with `entityPath` may declare `entityLifecycle: {unbound}`, the operation Core calls with a platform system ticket (`PluginSystemCallClaims`, its own signing domain) and `{entity, reason: "recalled" | "post_failed", serverMsgId?}`. `PluginManifest::warnings` reports a bound card without it.
- Envelope `displayName` (at most 64 bytes, written by the platform when it posts a card) and `ResolvedPluginView.unavailableReason` (`disabled`, `not_installed`, `no_permission`, `unsupported_platform`, `incompatible_version`, `catalog_unreachable`, and `unbound_copy` for a read-only copy, see `ResolvedPluginView::into_read_only`). `UiCatalog.unavailable` lists the tenant's plugins a viewer cannot use, with their reason.
- Page data: a `page` contribution may declare `source: {operation, input?, cursorPath?}`, a declared query whose result the page document is resolved against (without a source the page resolves against its arguments). A page is opened with arguments, the input the opening action bound (`{}` from an account entry); `input` maps operation inputs to pointers into them, and an argument the pointer does not find is left out (`UiPageSource::bind_input`). Paging follows the detail convention: `nextCursor` from `cursorPath`, the next page is the same input plus `cursor` (`UiPageSource::page_input`), appended with `append_page`. `UiCatalog::page` finds a page or says why it cannot be shown, and `UiContribution::resolve_page` resolves it. Page documents may hold one rows block: its `source` is the page source, it pages with the page (no `cursorPath` of its own), and the values of its filter inputs join the source input; `append_page` appends rows, dropping keys already shown. A page's row actions may open another page (a page target): the row's bound input (`rows[].actions[].input`) is that page's arguments, so a ledger row opens the packet it belongs to. Admin page row actions call admin operations only.
- Read-only command recovery: a client page action may declare `command: {slot, mode: "observe"}` with a declared `query` target and no form, confirmation or terminal-code override. The SDK reads the original authenticated intent and sends `CommandObservationRequest {commandId, capabilityId, payload}`. `CommandObservation<ObservedCommandResult>` must match that identity and the canonical original-payload digest; its result preserves the original `success` and `data`, including a terminal rejection. `NotFound`, invalid receipts and transport failures leave the intent unknown and unacknowledgeable. Admin documents reject `observe`: they have no authenticated client journal. Core emits the authorized `queryOperations` subset only for plugins offering this mode; absent metadata grants no observation authority.
- Selection limit: a choices block's `maxSelectionsPath` names a multiple choice's limit (a positive integer, capped at the option count); the resolved block carries `maxSelections`, and `selection_input` refuses a larger selection.
- Item badges: entries and rows blocks take `valueLabels: {path, values: {value: {label, tone?}}}`, read inside each entry or row; the resolved entry or row carries `badge: {text, tone}`, the label translated through `texts`, an unmapped value shown as sent and neutral.
- Admin Extension Contract: the top-level `admin` section (`crate::admin::PluginAdmin`) declares staff permissions (granted as `plugin:<pluginId>:<permission>`), admin operations (declared capability operations with the same `mode`, never used by a client contribution), navigation, pages (`list`, `detail`, `form`, `dashboard` with kit whitelisted widgets, resolved with `AdminPage::resolve` and `resolve_detail`), configuration UI hints, events and lifecycle (migrations, uninstall). It is the only console surface: `PluginManifest::admin_operations` and `admin_permission_for` read it alone.

The host bridge reads are `im.conversation.read`, `im.conversation.participants.read` (pages of up to 500), `im.conversation.members.check` (`{"userIds": [...]}`, up to 50 unique ids, answered with `{"members": [...], "nonMembers": [...]}` in request order) and `social.group.member.read`, each scoped to the conversation the ticket binds.

`SharedState` is the entity snapshot Core turns into `EVENT_MESSAGE_PLUGIN_STATE`. Plugins return it as `data.sharedState`, or publish it through the one host write capability `im.plugin.entity_state.publish` (`ResourceWriteRequest`). User tickets (`PluginCallClaims`), staff tickets for admin operations (`PluginStaffCallClaims`, permission `plugin:<pluginId>:<permission>` and the operator's stated `reason`, required by Core for an operation declaring `requiresReason`), platform system tickets for platform callbacks (`PluginSystemCallClaims`) and the plugin runtime credential (`PluginRuntimeClaims`, bound to one deployment and one exact request body) use separate signing domains, so none verifies as another. Configuration properties marked `x-flare-client-visible: true` form the subset clients receive with the catalog.

## Commands

```sh
cargo test
cargo run --bin plugin-manifest -- path/to/plugin.json
cargo run --bin plugin-schema -- ../../flare-plugin-devkit/schema/platform_plugin_manifest.schema.json
```

Client composition-time SDK package manifests remain a separate generated contract. Platform control persists signed originals and immutable digests; deployed services and tenant grants reference those versions. Marketplace queries do not belong in the runtime call path.

## License

Apache-2.0; see the repository LICENSE.


### Delegated plugin services

`serviceOperations` declares private target operations in `user` or `recovery` mode. These cannot also appear in client or staff contributions. A source declares `plugin.service.dispatch` on its originating operation and a mandatory compatible dependency on its target. Core validates both approved grants and policies, routes through operator-owned deployment metadata, reserves its shared invocation budget and signs an independent service ticket over canonical input. Its 30-second lifetime is bounded by both current leases. The target verifies exact operation, request, body, audience, configuration and revision; an opaque admission exposes immutable claims.

User delegation carries the original dedicated Core invocation ticket to Core's bridge, never a user bearer. Recovery carries only the source deployment credential and reaches only explicitly declared recovery operations. It may settle accepted obligations while disabled, but never select a new beneficiary or obtain new spend authority. Service-capable applications need the control plane's short projection renewal cycle. Revoked publisher/version trust fails closed; accepted wallet escrows also have wallet-owned deadline recovery.


### Admin workspaces

An `admin.navigation` entry may declare `pages` with independently authorized
members. Its existing `page` is the preferred landing page and must be included
among the members. Each member declares `page`, localized `label`, `placement`
(`tab` or `tool`, default `tab`), and optional `parentPage` for contextual tools.
References must exist, be unique within and across workspaces, and form no
parent cycles. The existing page and operation permissions remain authoritative;
opening a workspace never authorizes any other member or action.

The admin contract permits up to 96 operation declarations. Permissions,
navigation entries, pages and members of one workspace remain limited to 64
each. Query and command adapters may share the same page and permission;
their separate operation capacity does not add grants or visible entries.
The generated schema exposes the operation, permission, navigation and page
array bounds, and semantic validation also checks all references and authority.

The server filters member pages before choosing an accessible landing page.
Console role resources keep their original page/action identifiers and add
workspace labels for grouping. A previously published tenant menu targeting an
original member page can still resolve to that member. New publication offers
workspace roots, while tenant labels, icons, hierarchy and order
remain tenant-owned. An empty explicit role policy continues to deny access.


### Explicit organization management

An admin operation declares `scope: "organization"` when it changes an organization-wide policy;
ordinary operations default to `application`. One staff permission cannot mix the two scopes.
An application grant (`plugin:<plugin>:<permission>@<app>`) never grants organization authority;
the reviewed organization grant is `plugin:<plugin>:<permission>@organization:<organization>`.
The gateway signs the explicit organization target after checking that grant, Platform rechecks
fresh IAM authority and Catalog ownership, and Core binds the plugin staff ticket to the same
owner. Employee `org_id` is membership only. Platform staff must also name an explicit target.

The application remains the reviewed deployment anchor on organization console routes:
`/apps/<app>/organizations/<organization>/plugins/<plugin>/admin/<permission>/pages/<page>`.
The standard page, detail and action renderer uses the manifest's scope; no plugin business
logic lives in Console. `requiresReason` on a detail operation becomes a reason confirmation
before any protected data is fetched, and the reason is signed into the staff context.

Catalog projects immutable application ownership in `PluginProjection.organizationId` and
`PluginTenantState.organization_id`. Runtime assertions bind the app, deployment and owner
(`flare-plugin-runtime:<deployment>:<organization>`), and the shared runtime store rejects owner
reassignment across grants or deployments. Admitted user/staff/service contexts expose this
owner separately from employee membership. Unverified test callers have no owner unless an
explicit test fixture supplies one.
