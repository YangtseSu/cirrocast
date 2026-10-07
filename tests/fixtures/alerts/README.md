<!--
SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
SPDX-License-Identifier: GPL-3.0-or-later
-->

# Alert fixtures

Every file in this directory is **hand-written or hand-trimmed first-party data**, licensed
GPL-3.0-or-later like the rest of the repository, and listed under an explicit `REUSE.toml`
annotation. No live upstream payload is stored verbatim here: the files were written to the CAP 1.2
and source schemas (and, where a field shape was uncertain, checked against a live response without
copying it), which is what keeps the recorded-files rule and the licence rule compatible for
warning data whose redistribution terms are unclear.

| File | Feeds | Exercises |
|---|---|---|
| `nws-tornado.json` | NWS `alerts/active` | A live `Extreme` tornado warning plus a `messageType = Cancel` record that must be dropped |
| `meteoalarm-at.json` | MeteoAlarm EDR index | Two features: one whose polygon contains Vienna, one whose polygon does not |
| `meteoalarm-heat-cap.xml` | MeteoAlarm `hubLink` | Three `info` blocks (de, en-GB, fr), two areas, `parameter`/`eventCode` pairs, a `polygon` and a `circle` |
| `qweather-rainstorm.json` | QWeather `weatheralert/v1` | A CAP-triple alert with HTML in `description`, an alert whose severity is only in the colour, and a `cancel` record |
| `wmoswic-index.json` | WMO SWIC WFS index | One feature with `capurl`, one boundary row with neither link |
| `wmoswic-cap.xml` | WMO SWIC CAP document | Two `info` blocks (`en-US`, `zh-CN`) so the locale rule is testable |
| `fpas-area.json` | FPAS `/alert/area` | One Met document, one `category = Geo` document, one `Cancel` document |
| `fpas-gale.xml` | FPAS `/alert/<uuid>` | A Met polygon containing the fixture point |
| `fpas-quake.xml` | FPAS `/alert/<uuid>` | A non-met document that must be dropped by `category` |
| `fpas-cancel.xml` | FPAS `/alert/<uuid>` | A `msgType = Cancel` document that must be dropped |
| `hko-warnsum.json` | HKO `warnsum` | A `T8` cyclone signal, an Amber rainstorm warning and a `CANCEL` entry |
| `hko-warninginfo.json` | HKO `warningInfo` | Statement text per warning code |
| `hko-warnsum-empty.json` | HKO `warnsum` | The `{}` no-warnings case |
| `cap-truncated.xml` | shared CAP reader | A document that ends inside an element (must be an upstream error) |
| `cap-no-event.xml` | shared CAP reader | A well-formed document whose only `info` block has no `event` (must be unusable) |
