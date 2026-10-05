// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! The CAP v1.2 reader shared by the `MeteoAlarm`, WMO `SWIC` and `FPAS` adapters.
//!
//! Common Alerting Protocol documents are the federation lingua franca: three of this step's six
//! sources serve them (`MeteoAlarm`'s `hubLink`, WMO `SWIC`'s `capurl`, `FPAS`'s `/alert/<uuid>`), and
//! parsing them once is what keeps the alert model free of per-source quirks. The **parsed subset
//! is deliberately fixed** and is exactly this list:
//!
//! * alert level: `identifier`, `sender`, `sent`, `status`, `msgType`, `scope`, `references`;
//! * info level: `language`, `category`, `event`, `responseType`, `urgency`, `severity`,
//!   `certainty`, `effective`, `onset`, `expires`, `senderName`, `headline`, `description`,
//!   `instruction`, `web`, `contact`, `parameter`, `eventCode`;
//! * area level: `areaDesc`, `polygon`, `circle`, `geocode`, `altitude`, `ceiling`.
//!
//! Anything else is ignored, so an agency adding extensions cannot break the reader — and cannot
//! smuggle a field into the model either. The reader is a hand-written state machine over
//! `quick-xml`'s pull events; there is no serde mapping, which is what keeps that list literal.
//!
//! Two robustness rules, both driven by real federation traffic:
//!
//! * an XML syntax error is an upstream error (the caller decides whether the whole source fails
//!   or the one document is skipped);
//! * an `info` block without an `event` is unusable — it cannot be rendered or de-duplicated — so
//!   it is skipped, and a document whose blocks are all unusable is an upstream error.
//!
//! Multiple `info` blocks are the norm: the same warning is repeated per language and sometimes
//! per audience. [`alerts_from_cap`] runs the locale-match rule (the block matching the requested
//! language wins, else the first usable one) and unions the areas of every block.

use chrono::{DateTime, FixedOffset, NaiveDateTime, TimeZone as _, Utc};
use quick_xml::Reader;
use quick_xml::events::Event;

use crate::error::{Error, Result};
use crate::model::{Alert, AlertSource, Certainty, Severity, Urgency};

/// One alert-level CAP document, reduced to the parsed subset.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct CapDocument {
    /// CAP `identifier`.
    pub identifier: String,
    /// CAP `sender` (the machine sender, not the human agency name).
    pub sender: Option<String>,
    /// CAP `sent`.
    pub sent: Option<DateTime<FixedOffset>>,
    /// CAP `status`.
    pub status: Option<String>,
    /// CAP `msgType`.
    pub msg_type: Option<String>,
    /// CAP `scope`.
    pub scope: Option<String>,
    /// CAP `references`.
    pub references: Option<String>,
    /// The `info` blocks, in document order.
    pub infos: Vec<CapInfo>,
}

/// One `info` block.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct CapInfo {
    /// CAP `language`.
    pub language: Option<String>,
    /// CAP `category` (repeatable).
    pub categories: Vec<String>,
    /// CAP `event`.
    pub event: String,
    /// CAP `responseType` (repeatable).
    pub responses: Vec<String>,
    /// CAP `urgency`.
    pub urgency: String,
    /// CAP `severity`.
    pub severity: String,
    /// CAP `certainty`.
    pub certainty: String,
    /// CAP `effective`.
    pub effective: Option<DateTime<FixedOffset>>,
    /// CAP `onset`.
    pub onset: Option<DateTime<FixedOffset>>,
    /// CAP `expires`.
    ///
    /// An `<expires>` that does not parse is treated as absent: the alert is kept (a live warning
    /// is never dropped over a malformed date) and reads as having no stated end. That is this
    /// module's single behaviour for an unparsable expiry — it is not silently invented, and
    /// [`Alert::effective_end`] documents the same "no reported end means live" rule.
    pub expires: Option<DateTime<FixedOffset>>,
    /// CAP `senderName`.
    pub sender_name: Option<String>,
    /// CAP `headline`.
    pub headline: Option<String>,
    /// CAP `description`.
    pub description: Option<String>,
    /// CAP `instruction`.
    pub instruction: Option<String>,
    /// CAP `web`.
    pub web: Option<String>,
    /// CAP `contact`.
    pub contact: Option<String>,
    /// CAP `parameter` pairs.
    pub parameters: Vec<(String, String)>,
    /// CAP `eventCode` pairs.
    pub event_codes: Vec<(String, String)>,
    /// The `area` blocks.
    pub areas: Vec<CapArea>,
}

impl CapInfo {
    /// Whether this block can be turned into an alert at all.
    #[must_use]
    pub fn is_usable(&self) -> bool {
        !self.event.trim().is_empty()
    }
}

/// One `area` block.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct CapArea {
    /// CAP `areaDesc`.
    pub area_desc: String,
    /// CAP `polygon` strings (`lat,lon lat,lon …`).
    pub polygons: Vec<String>,
    /// CAP `circle` strings (`lat,lon radius-km`).
    pub circles: Vec<String>,
    /// CAP `geocode` pairs (`valueName` → `value`).
    pub geocodes: Vec<(String, String)>,
    /// CAP `altitude`, in metres.
    pub altitude: Option<f64>,
    /// CAP `ceiling`, in metres.
    pub ceiling: Option<f64>,
}

/// The fields of an `info` or `area` block the parser can tell apart.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Leaf {
    Identifier,
    Sender,
    Sent,
    Status,
    MsgType,
    Scope,
    References,
    Language,
    Category,
    Event,
    ResponseType,
    Urgency,
    Severity,
    Certainty,
    Effective,
    Onset,
    Expires,
    SenderName,
    Headline,
    Description,
    Instruction,
    Web,
    Contact,
    AreaDesc,
    Polygon,
    Circle,
    Altitude,
    Ceiling,
    ValueName,
    Value,
}

/// The element name as [`Leaf`], if it is one this reader collects.
fn leaf_of(name: &str) -> Option<Leaf> {
    Some(match name {
        "identifier" => Leaf::Identifier,
        "sender" => Leaf::Sender,
        "sent" => Leaf::Sent,
        "status" => Leaf::Status,
        "msgType" => Leaf::MsgType,
        "scope" => Leaf::Scope,
        "references" => Leaf::References,
        "language" => Leaf::Language,
        "category" => Leaf::Category,
        "event" => Leaf::Event,
        "responseType" => Leaf::ResponseType,
        "urgency" => Leaf::Urgency,
        "severity" => Leaf::Severity,
        "certainty" => Leaf::Certainty,
        "effective" => Leaf::Effective,
        "onset" => Leaf::Onset,
        "expires" => Leaf::Expires,
        "senderName" => Leaf::SenderName,
        "headline" => Leaf::Headline,
        "description" => Leaf::Description,
        "instruction" => Leaf::Instruction,
        "web" => Leaf::Web,
        "contact" => Leaf::Contact,
        "areaDesc" => Leaf::AreaDesc,
        "polygon" => Leaf::Polygon,
        "circle" => Leaf::Circle,
        "altitude" => Leaf::Altitude,
        "ceiling" => Leaf::Ceiling,
        "valueName" => Leaf::ValueName,
        "value" => Leaf::Value,
        _ => return None,
    })
}

/// Which repeatable pair list a `valueName`/`value` container fills.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PairKind {
    Parameter,
    EventCode,
    Geocode,
}

/// The pair currently being collected.
#[derive(Debug, Default)]
struct Pair {
    kind: Option<PairKind>,
    name: String,
    value: String,
}

/// Parses one CAP document.
///
/// A syntax error names the source and the parser's own message; the caller decides whether the
/// whole source fails or one federated document is skipped.
pub fn parse_cap(xml: &str, source: AlertSource) -> Result<CapDocument> {
    let mut reader = Reader::from_str(xml);
    let mut parser = Parser {
        document: CapDocument::default(),
        path: Vec::new(),
        field: None,
        buffer: String::new(),
        pair: Pair::default(),
    };
    loop {
        match reader.read_event() {
            Err(error) => {
                return Err(upstream(
                    source,
                    format!("cannot parse the CAP document: {error}"),
                ));
            }
            Ok(Event::Eof) => break,
            Ok(Event::Start(event)) => {
                let name = local_name(&event);
                parser.start(&name);
            }
            Ok(Event::Empty(event)) => {
                let name = local_name(&event);
                parser.start(&name);
                parser.end(&name);
            }
            Ok(Event::Text(text)) => {
                if parser.field.is_some() {
                    parser.buffer.push_str(&text.xml10_content());
                }
            }
            Ok(Event::CData(text)) => {
                if parser.field.is_some() {
                    parser.buffer.push_str(&text.into_inner());
                }
            }
            Ok(Event::GeneralRef(reference)) => {
                if parser.field.is_some() {
                    resolve_reference(&reference.into_inner(), &mut parser.buffer);
                }
            }
            Ok(Event::End(event)) => {
                let name = event.local_name().as_ref().to_owned();
                parser.end(&name);
            }
            Ok(_) => {}
        }
    }
    if !parser.path.is_empty() {
        return Err(upstream(
            source,
            format!(
                "the CAP document ends with unclosed elements: {}",
                parser.path.join("/")
            ),
        ));
    }
    if parser.document.infos.is_empty() && parser.document.identifier.is_empty() {
        return Err(upstream(
            source,
            "the CAP document carries neither an identifier nor an info block".to_owned(),
        ));
    }
    Ok(parser.document)
}

/// The parsed-document state machine.
struct Parser {
    document: CapDocument,
    path: Vec<String>,
    field: Option<Leaf>,
    buffer: String,
    pair: Pair,
}

impl Parser {
    fn start(&mut self, name: &str) {
        match name {
            "info" => self.document.infos.push(CapInfo::default()),
            "area" => {
                if let Some(info) = self.document.infos.last_mut() {
                    info.areas.push(CapArea::default());
                }
            }
            "parameter" => {
                self.pair = Pair {
                    kind: Some(PairKind::Parameter),
                    ..Pair::default()
                }
            }
            "eventCode" => {
                self.pair = Pair {
                    kind: Some(PairKind::EventCode),
                    ..Pair::default()
                }
            }
            "geocode" => {
                self.pair = Pair {
                    kind: Some(PairKind::Geocode),
                    ..Pair::default()
                }
            }
            other => {
                self.field = leaf_of(other);
                self.buffer.clear();
            }
        }
        self.path.push(name.to_owned());
    }

    fn end(&mut self, name: &str) {
        self.path.pop();
        match name {
            "parameter" | "eventCode" | "geocode" => {
                let pair = std::mem::take(&mut self.pair);
                if let Some(kind) = pair.kind {
                    match kind {
                        PairKind::Parameter => {
                            if let Some(info) = self.document.infos.last_mut() {
                                info.parameters.push((pair.name, pair.value));
                            }
                        }
                        PairKind::EventCode => {
                            if let Some(info) = self.document.infos.last_mut() {
                                info.event_codes.push((pair.name, pair.value));
                            }
                        }
                        PairKind::Geocode => {
                            if let Some(area) = current_area(&mut self.document.infos) {
                                area.geocodes.push((pair.name, pair.value));
                            }
                        }
                    }
                }
            }
            other => {
                let Some(leaf) = leaf_of(other) else {
                    return;
                };
                if self.field != Some(leaf) {
                    return;
                }
                let text = std::mem::take(&mut self.buffer);
                let text = text.trim().to_owned();
                self.field = None;
                self.commit(leaf, text);
            }
        }
    }

    /// Files one collected leaf value into the block its position implies.
    fn commit(&mut self, leaf: Leaf, text: String) {
        let in_area = self.path.iter().any(|name| name == "area");
        let in_info = self.path.iter().any(|name| name == "info");

        if leaf == Leaf::ValueName {
            self.pair.name = text;
            return;
        }
        if leaf == Leaf::Value {
            self.pair.value = text;
            return;
        }
        if in_area {
            if let Some(area) = current_area(&mut self.document.infos) {
                match leaf {
                    Leaf::AreaDesc => area.area_desc = text,
                    Leaf::Polygon => area.polygons.push(text),
                    Leaf::Circle => area.circles.push(text),
                    Leaf::Altitude => area.altitude = text.parse().ok(),
                    Leaf::Ceiling => area.ceiling = text.parse().ok(),
                    _ => {}
                }
            }
            return;
        }
        if in_info {
            if let Some(info) = self.document.infos.last_mut() {
                match leaf {
                    Leaf::Language => info.language = non_empty(&text),
                    Leaf::Category => info.categories.push(text),
                    Leaf::Event => info.event = text,
                    Leaf::ResponseType => info.responses.push(text),
                    Leaf::Urgency => info.urgency = text,
                    Leaf::Severity => info.severity = text,
                    Leaf::Certainty => info.certainty = text,
                    Leaf::Effective => info.effective = instant(&text),
                    Leaf::Onset => info.onset = instant(&text),
                    Leaf::Expires => info.expires = instant(&text),
                    Leaf::SenderName => info.sender_name = non_empty(&text),
                    Leaf::Headline => info.headline = non_empty(&text),
                    Leaf::Description => info.description = non_empty(&text),
                    Leaf::Instruction => info.instruction = non_empty(&text),
                    Leaf::Web => info.web = non_empty(&text),
                    Leaf::Contact => info.contact = non_empty(&text),
                    _ => {}
                }
            }
            return;
        }
        match leaf {
            Leaf::Identifier => self.document.identifier = text,
            Leaf::Sender => self.document.sender = non_empty(&text),
            Leaf::Sent => self.document.sent = instant(&text),
            Leaf::Status => self.document.status = non_empty(&text),
            Leaf::MsgType => self.document.msg_type = non_empty(&text),
            Leaf::Scope => self.document.scope = non_empty(&text),
            Leaf::References => self.document.references = non_empty(&text),
            _ => {}
        }
    }
}

/// The last area of the last info block, which is where area-level fields land.
fn current_area(infos: &mut [CapInfo]) -> Option<&mut CapArea> {
    infos.last_mut()?.areas.last_mut()
}

/// `Some(trimmed)` unless the text is empty.
fn non_empty(text: &str) -> Option<String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_owned())
    }
}

/// Whether a CAP `status` describes a message that is not a live warning: `Test` (a drill),
/// `Exercise` (a planned exercise) or `Draft` (an unpublished draft).
fn is_not_actual(value: &str) -> bool {
    let value = value.trim();
    value.eq_ignore_ascii_case("test")
        || value.eq_ignore_ascii_case("exercise")
        || value.eq_ignore_ascii_case("draft")
}

/// The element's local name (namespace prefixes are dropped).
fn local_name(event: &quick_xml::events::BytesStart<'_>) -> String {
    event.local_name().as_ref().to_owned()
}

/// A CAP `dateTime`: RFC 3339 (relaxed) with an offset, else a plain local timestamp read as UTC.
///
/// The relaxed form accepts the ISO-8601 spellings real agencies emit: `T` or a space between date
/// and time, optional fractional seconds, `Z`/`UTC`, and an offset written with or without its
/// colon (`+02:00` and `+0200`). Anything unparsable yields `None`; the caller decides what a
/// missing instant means — see [`CapInfo::expires`].
pub(crate) fn instant(text: &str) -> Option<DateTime<FixedOffset>> {
    let text = text.trim();
    if let Ok(at) = text.parse::<DateTime<FixedOffset>>() {
        return Some(at);
    }
    // The relaxed parser requires seconds; accept `06:00+0200` too.
    for format in ["%Y-%m-%dT%H:%M%z", "%Y-%m-%d %H:%M%z"] {
        if let Ok(at) = DateTime::parse_from_str(text, format) {
            return Some(at);
        }
    }
    for format in ["%Y-%m-%dT%H:%M:%S", "%Y-%m-%dT%H:%M"] {
        if let Ok(naive) = NaiveDateTime::parse_from_str(text, format) {
            return Some(Utc.from_utc_datetime(&naive).fixed_offset());
        }
    }
    None
}

/// Resolves a predefined or numeric XML entity reference.
///
/// `quick-xml` reports `&amp;` and friends as [`Event::GeneralRef`] rather than folding them into
/// the text, so the five predefined entities and the numeric forms are resolved here. An
/// unresolvable reference is dropped — an undeclared entity is ill-formed XML without a DTD, and
/// dropping it keeps one bad character from failing a whole warning document.
fn resolve_reference(name: &str, out: &mut String) {
    match name {
        "amp" => out.push('&'),
        "lt" => out.push('<'),
        "gt" => out.push('>'),
        "quot" => out.push('"'),
        "apos" => out.push('\''),
        _ => {
            let code =
                name.strip_prefix('#')
                    .and_then(|rest| match rest.strip_prefix(['x', 'X']) {
                        Some(hex) => u32::from_str_radix(hex, 16).ok(),
                        None => rest.parse().ok(),
                    });
            if let Some(character) = code.and_then(char::from_u32) {
                out.push(character);
            }
        }
    }
}

/// The upstream error for a malformed CAP document.
fn upstream(source: AlertSource, message: String) -> Error {
    Error::Upstream {
        provider: source.as_str().to_owned(),
        status: None,
        message,
    }
}

/// Turns one parsed document into the model, choosing the `info` block by locale.
///
/// * `msgType = Cancel` yields no alerts: a cancellation removes a warning, it does not add one.
/// * a `status` of `Test`, `Exercise` or `Draft` yields no alerts either: those are drills and
///   drafts, not live warnings, and rendering one as a warning would be a false alarm (the `NWS`
///   `GeoJSON` path drops its `Test` status the same way).
/// * the block whose `language` matches `language` wins (exact tag, then primary subtag); else the
///   first usable block.
/// * areas are unioned across **every** block, de-duplicated in document order, because the area
///   set is language-independent and a block may carry a subset.
/// * a document with no usable block is an upstream error.
pub fn alerts_from_cap(
    document: &CapDocument,
    source: AlertSource,
    language: &str,
) -> Result<Vec<Alert>> {
    if document
        .msg_type
        .as_deref()
        .is_some_and(|kind| kind.eq_ignore_ascii_case("Cancel"))
    {
        return Ok(Vec::new());
    }
    if document.status.as_deref().is_some_and(is_not_actual) {
        return Ok(Vec::new());
    }
    if document.identifier.trim().is_empty() {
        return Err(upstream(
            source,
            "the CAP document has no identifier".to_owned(),
        ));
    }
    let info = pick_info(&document.infos, language).ok_or_else(|| {
        upstream(
            source,
            format!(
                "CAP document `{}` has no info block with an event",
                document.identifier
            ),
        )
    })?;

    let areas = union_areas(&document.infos);
    let headline = info
        .headline
        .clone()
        .filter(|text| !text.trim().is_empty())
        .unwrap_or_else(|| info.event.clone());
    Ok(vec![Alert {
        id: document.identifier.trim().to_owned(),
        source,
        event: info.event.trim().to_owned(),
        severity: Severity::from_cap(&info.severity),
        urgency: Urgency::from_cap(&info.urgency),
        certainty: Certainty::from_cap(&info.certainty),
        onset: info.onset.or(info.effective),
        expires: info.expires,
        ends: None,
        areas,
        headline,
        description: info.description.clone(),
        instruction: info.instruction.clone(),
        sender: info.sender_name.clone().or_else(|| document.sender.clone()),
    }])
}

/// The `info` block for `language`: exact tag first, then primary subtag, then the first usable
/// block; `None` when no block has an event.
fn pick_info<'a>(infos: &'a [CapInfo], language: &str) -> Option<&'a CapInfo> {
    let wanted = language.trim().to_ascii_lowercase().replace('_', "-");
    let primary = wanted.split('-').next().unwrap_or_default();
    let usable = || infos.iter().filter(|info| info.is_usable());
    usable()
        .find(|info| {
            info.language
                .as_deref()
                .is_some_and(|tag| tag.eq_ignore_ascii_case(&wanted))
        })
        .or_else(|| {
            usable().find(|info| {
                info.language.as_deref().is_some_and(|tag| {
                    tag.to_ascii_lowercase()
                        .split('-')
                        .next()
                        .is_some_and(|value| value == primary)
                })
            })
        })
        .or_else(|| usable().next())
}

/// Every `areaDesc` across every `info` block, de-duplicated in document order.
fn union_areas(infos: &[CapInfo]) -> Vec<String> {
    let mut areas = Vec::new();
    for info in infos {
        for area in &info.areas {
            let text = area.area_desc.trim();
            if !text.is_empty() && !areas.iter().any(|seen| seen == text) {
                areas.push(text.to_owned());
            }
        }
    }
    areas
}

#[cfg(test)]
mod tests {
    use super::{alerts_from_cap, parse_cap};
    use crate::model::{AlertSource, Certainty, Severity, Urgency};

    /// A hand-written three-language CAP document with two areas and pairs.
    const MULTI: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<alert xmlns="urn:oasis:names:tc:emergency:cap:1.2">
  <identifier>test-heat-001</identifier>
  <sender>op@example.gov</sender>
  <sent>2026-10-03T06:00:00+02:00</sent>
  <status>Actual</status>
  <msgType>Alert</msgType>
  <scope>Public</scope>
  <references>op@example.gov,test-heat-000,2026-10-02T06:00:00+02:00</references>
  <info>
    <language>de</language>
    <category>Met</category>
    <event>Hitzewarnung</event>
    <urgency>Expected</urgency>
    <severity>Severe</severity>
    <certainty>Likely</certainty>
    <effective>2026-10-03T08:00:00+02:00</effective>
    <onset>2026-10-03T10:00:00+02:00</onset>
    <expires>2026-10-04T18:00:00+02:00</expires>
    <senderName>Bundeswarnzentrale</senderName>
    <headline>Schwere Hitzewelle</headline>
    <description>Es wird hei&#223; &amp; schw&#252;l.</description>
    <instruction>Suchen Sie Schatten.</instruction>
    <web>https://example.gov/heat</web>
    <contact>op@example.gov</contact>
    <parameter><valueName>eventId</valueName><value>42</value></parameter>
    <eventCode><valueName>SAME</valueName><value>EHT</value></eventCode>
    <area>
      <areaDesc>Wien</areaDesc>
      <polygon>48.1,16.2 48.3,16.5 48.0,16.6 48.1,16.2</polygon>
      <circle>48.2,16.37 25</circle>
      <geocode><valueName>NUTS</valueName><value>AT13</value></geocode>
      <altitude>200</altitude>
      <ceiling>5000</ceiling>
    </area>
  </info>
  <info>
    <language>en-GB</language>
    <category>Met</category>
    <event>Heat warning</event>
    <urgency>Expected</urgency>
    <severity>Severe</severity>
    <certainty>Likely</certainty>
    <onset>2026-10-03T09:00:00Z</onset>
    <expires>2026-10-04T16:00:00Z</expires>
    <senderName>Federal Warning Centre</senderName>
    <headline>Severe heat wave</headline>
    <description>It will be hot.</description>
    <instruction>Find shade.</instruction>
    <area><areaDesc>Vienna</areaDesc></area>
  </info>
  <info>
    <language>fr</language>
    <category>Met</category>
    <event>Alerte chaleur</event>
    <severity>Severe</severity>
  </info>
</alert>"#;

    #[test]
    fn every_listed_field_is_parsed() {
        let document = parse_cap(MULTI, AlertSource::MeteoAlarm).expect("the document parses");
        assert_eq!(document.identifier, "test-heat-001");
        assert_eq!(document.sender.as_deref(), Some("op@example.gov"));
        assert_eq!(document.status.as_deref(), Some("Actual"));
        assert_eq!(document.msg_type.as_deref(), Some("Alert"));
        assert_eq!(document.scope.as_deref(), Some("Public"));
        assert!(document.references.is_some());
        assert_eq!(document.infos.len(), 3);

        let german = &document.infos[0];
        assert_eq!(german.language.as_deref(), Some("de"));
        assert_eq!(german.event, "Hitzewarnung");
        assert_eq!(german.severity, "Severe");
        assert_eq!(german.urgency, "Expected");
        assert_eq!(german.certainty, "Likely");
        assert_eq!(german.sender_name.as_deref(), Some("Bundeswarnzentrale"));
        assert_eq!(german.headline.as_deref(), Some("Schwere Hitzewelle"));
        assert_eq!(
            german.description.as_deref(),
            Some("Es wird heiß & schwül.")
        );
        assert_eq!(german.instruction.as_deref(), Some("Suchen Sie Schatten."));
        assert_eq!(german.web.as_deref(), Some("https://example.gov/heat"));
        assert_eq!(german.contact.as_deref(), Some("op@example.gov"));
        assert_eq!(german.parameters, [("eventId".to_owned(), "42".to_owned())]);
        assert_eq!(german.event_codes, [("SAME".to_owned(), "EHT".to_owned())]);
        assert!(german.expires.is_some());

        let area = &german.areas[0];
        assert_eq!(area.area_desc, "Wien");
        assert_eq!(area.polygons.len(), 1);
        assert_eq!(area.circles, ["48.2,16.37 25"]);
        assert_eq!(area.geocodes, [("NUTS".to_owned(), "AT13".to_owned())]);
        assert_eq!(area.altitude, Some(200.0));
        assert_eq!(area.ceiling, Some(5000.0));
    }

    #[test]
    fn the_requested_language_wins_exact_tag_then_primary() {
        let document = parse_cap(MULTI, AlertSource::MeteoAlarm).expect("the document parses");
        let english =
            alerts_from_cap(&document, AlertSource::MeteoAlarm, "en-US").expect("a usable block");
        assert_eq!(english[0].event, "Heat warning");
        assert_eq!(english[0].sender.as_deref(), Some("Federal Warning Centre"));

        let german =
            alerts_from_cap(&document, AlertSource::MeteoAlarm, "de").expect("a usable block");
        assert_eq!(german[0].event, "Hitzewarnung");
        assert_eq!(german[0].severity, Severity::Severe);
        assert_eq!(german[0].urgency, Urgency::Expected);
        assert_eq!(german[0].certainty, Certainty::Likely);

        // No zh block: the first usable one is used rather than failing the document.
        let fallback =
            alerts_from_cap(&document, AlertSource::MeteoAlarm, "zh-CN").expect("a usable block");
        assert_eq!(fallback[0].event, "Hitzewarnung");
    }

    #[test]
    fn areas_are_unioned_across_blocks_in_document_order() {
        let document = parse_cap(MULTI, AlertSource::MeteoAlarm).expect("the document parses");
        let alerts = alerts_from_cap(&document, AlertSource::MeteoAlarm, "en-GB").expect("usable");
        assert_eq!(alerts[0].areas, ["Wien", "Vienna"]);
    }

    #[test]
    fn a_cancellation_yields_no_alerts() {
        let cancel = r"<alert><identifier>x-1</identifier><msgType>Cancel</msgType>
            <info><event>Heat warning</event><language>en</language></info></alert>";
        let document = parse_cap(cancel, AlertSource::Fpas).expect("the document parses");
        let alerts = alerts_from_cap(&document, AlertSource::Fpas, "en")
            .expect("cancellations are not errors");
        assert_eq!(alerts.len(), 0, "a cancellation is not an alert");
    }

    #[test]
    fn truncated_xml_and_eventless_info_blocks_are_upstream_errors() {
        let error = parse_cap("<alert><identifier>x</identifier>", AlertSource::Fpas)
            .expect_err("truncated XML is an error");
        assert!(error.to_string().contains("CAP"), "{error}");

        let no_event = r"<alert><identifier>x-2</identifier>
            <info><language>en</language><severity>Minor</severity></info></alert>";
        let document = parse_cap(no_event, AlertSource::Fpas).expect("the XML is well formed");
        let error = alerts_from_cap(&document, AlertSource::Fpas, "en")
            .expect_err("a document without an event is unusable");
        assert!(
            error.to_string().contains("no info block with an event"),
            "{error}"
        );
    }

    #[test]
    fn a_bare_identifier_still_parses() {
        let document = parse_cap(
            "<alert><identifier>only</identifier></alert>",
            AlertSource::Hko,
        )
        .expect("the document parses");
        assert_eq!(document.identifier, "only");
        assert_eq!(document.infos.len(), 0);
    }

    #[test]
    fn test_exercise_and_draft_statuses_yield_no_alerts() {
        for status in ["Test", "Exercise", "Draft", "test"] {
            let xml = format!(
                r"<alert><identifier>drill-1</identifier><status>{status}</status>
                   <msgType>Alert</msgType>
                   <info><language>en</language><event>Severe Thunderstorm Warning</event>
                   <severity>Extreme</severity></info></alert>"
            );
            let document = parse_cap(&xml, AlertSource::Fpas).expect("the document parses");
            let alerts = alerts_from_cap(&document, AlertSource::Fpas, "en")
                .expect("a drill is not an error");
            assert_eq!(alerts.len(), 0, "status {status} is not a live warning");
        }

        // `Actual` still produces its alert.
        let actual = r"<alert><identifier>x-3</identifier><status>Actual</status>
            <msgType>Alert</msgType>
            <info><language>en</language><event>Gale</event>
            <severity>Severe</severity></info></alert>";
        let document = parse_cap(actual, AlertSource::Fpas).expect("the document parses");
        assert_eq!(
            alerts_from_cap(&document, AlertSource::Fpas, "en")
                .expect("a live warning")
                .len(),
            1
        );
    }

    #[test]
    fn an_iso_basic_offset_expiry_parses_and_an_unparsable_one_keeps_the_alert() {
        // `+0200` (no colon) is the ISO-8601 basic offset real CAP documents carry.
        let basic = r"<alert><identifier>basic-1</identifier><status>Actual</status>
            <msgType>Alert</msgType>
            <info><language>en</language><event>Heat warning</event><severity>Severe</severity>
            <expires>2026-10-04T18:00:00+0200</expires></info></alert>";
        let document = parse_cap(basic, AlertSource::Fpas).expect("the document parses");
        let alerts = alerts_from_cap(&document, AlertSource::Fpas, "en").expect("a usable block");
        assert_eq!(
            alerts[0].expires.map(|at| at.to_rfc3339()),
            Some("2026-10-04T18:00:00+02:00".to_owned())
        );

        // An unparsable expiry is absent, not fatal: the alert is kept and reads as open-ended.
        let broken = r"<alert><identifier>broken-1</identifier><status>Actual</status>
            <msgType>Alert</msgType>
            <info><language>en</language><event>Heat warning</event><severity>Severe</severity>
            <expires>tomorrow-ish</expires></info></alert>";
        let document = parse_cap(broken, AlertSource::Fpas).expect("the document parses");
        let alerts = alerts_from_cap(&document, AlertSource::Fpas, "en").expect("a usable block");
        assert_eq!(alerts.len(), 1);
        assert_eq!(alerts[0].expires, None);
    }
}
