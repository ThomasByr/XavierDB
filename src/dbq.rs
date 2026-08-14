//! MongoDB operations: JSON<->BSON conversion (with extended-JSON support),
//! cursor encoding/decoding and keyset pagination.

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use bson::doc;
use bson::oid::ObjectId;
use bson::{Bson, DateTime, Document};
use futures::TryStreamExt;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::{ApiError, ApiErrorKind};
use crate::state::AppState;

// ---------------------------------------------------------------------------
// JSON <-> BSON
// ---------------------------------------------------------------------------

/// Convert a client-supplied JSON value into BSON, understanding the MongoDB
/// extended-JSON tokens: $oid, $date, $numberLong, $numberInt, $numberDouble,
/// $numberDecimal, $binary, $regex, $timestamp, $minKey, $maxKey.
pub fn json_to_bson(v: &Value) -> Result<Bson, String> {
    match v {
        Value::Null => Ok(Bson::Null),
        Value::Bool(b) => Ok(Bson::Boolean(*b)),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Ok(Bson::Int64(i))
            } else if let Some(u) = n.as_u64() {
                // BSON has no unsigned 64-bit type; values above i64::MAX are
                // stored as Decimal128 (casting would silently wrap negative
                // and corrupt queries/inserts).
                if let Ok(i) = i64::try_from(u) {
                    Ok(Bson::Int64(i))
                } else {
                    u.to_string()
                        .parse::<bson::Decimal128>()
                        .map(Bson::Decimal128)
                        .map_err(|e| format!("bad u64: {e}"))
                }
            } else {
                Ok(Bson::Double(n.as_f64().unwrap_or(0.0)))
            }
        }
        Value::String(s) => Ok(Bson::String(s.clone())),
        Value::Array(a) => a
            .iter()
            .map(json_to_bson)
            .collect::<Result<Vec<_>, _>>()
            .map(Bson::Array),
        Value::Object(m) => {
            // { $regex, $options } — the two-key form must still become a
            // regular expression (a literal document would be unmatchable)
            if let Some(Value::String(re)) = m.get("$regex") {
                if m.len() == 1 || (m.len() == 2 && m.contains_key("$options")) {
                    let opts = m
                        .get("$options")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string();
                    return Ok(Bson::RegularExpression(bson::Regex {
                        pattern: re.clone(),
                        options: opts,
                    }));
                }
            }
            if m.len() == 1 {
                if let Some(Value::String(s)) = m.get("$oid") {
                    return ObjectId::parse_str(s)
                        .map(Bson::ObjectId)
                        .map_err(|e| format!("bad $oid: {e}"));
                }
                if let Some(v) = m.get("$date") {
                    return match v {
                        Value::String(s) => DateTime::parse_rfc3339_str(s)
                            .map(Bson::DateTime)
                            .map_err(|e| format!("bad $date: {e}")),
                        Value::Object(o) if o.get("$numberLong").is_some() => {
                            parse_i64(o["$numberLong"].as_str().unwrap_or(""))
                                .map(|ms| Bson::DateTime(DateTime::from_millis(ms)))
                                .ok_or_else(|| "bad $date millis".into())
                        }
                        Value::Number(n) => n
                            .as_i64()
                            .map(|ms| Bson::DateTime(DateTime::from_millis(ms)))
                            .ok_or_else(|| "bad $date millis".into()),
                        _ => Err("bad $date".into()),
                    };
                }
                if let Some(Value::String(s)) = m.get("$numberLong") {
                    return parse_i64(s)
                        .map(Bson::Int64)
                        .ok_or_else(|| "bad $numberLong".into());
                }
                if let Some(Value::String(s)) = m.get("$numberInt") {
                    return s
                        .parse::<i32>()
                        .map(Bson::Int32)
                        .map_err(|_| "bad $numberInt".into());
                }
                if let Some(Value::String(s)) = m.get("$numberDouble") {
                    return s
                        .parse::<f64>()
                        .map(Bson::Double)
                        .map_err(|_| "bad $numberDouble".into());
                }
                if let Some(Value::String(s)) = m.get("$numberDecimal") {
                    return s
                        .parse::<bson::Decimal128>()
                        .map(Bson::Decimal128)
                        .map_err(|_| "bad $numberDecimal".into());
                }
                if let Some(o) = m.get("$binary") {
                    if let (Some(Value::String(b64)), Some(Value::String(st))) =
                        (o.get("base64"), o.get("subType"))
                    {
                        let bytes = base64::engine::general_purpose::STANDARD
                            .decode(b64)
                            .map_err(|_| "bad $binary base64".to_string())?;
                        let subtype = u8::from_str_radix(st, 16).unwrap_or(0);
                        let subtype = match subtype {
                            0 => bson::spec::BinarySubtype::Generic,
                            _ => bson::spec::BinarySubtype::UserDefined(subtype),
                        };
                        return Ok(Bson::Binary(bson::Binary { subtype, bytes }));
                    }
                }
                if let Some(Value::String(re)) = m.get("$regex") {
                    let opts = m
                        .get("$options")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string();
                    return Ok(Bson::RegularExpression(bson::Regex {
                        pattern: re.clone(),
                        options: opts,
                    }));
                }
                if let Some(o) = m.get("$timestamp") {
                    let Value::Object(o) = o else {
                        return Err("bad $timestamp".into());
                    };
                    let t = o
                        .get("t")
                        .and_then(Value::as_i64)
                        .filter(|v| *v >= 0 && *v <= u32::MAX as i64)
                        .ok_or_else(|| String::from("bad $timestamp t"))?
                        as u32;
                    let i = o
                        .get("i")
                        .and_then(Value::as_i64)
                        .filter(|v| *v >= 0 && *v <= u32::MAX as i64)
                        .ok_or_else(|| String::from("bad $timestamp i"))?
                        as u32;
                    return Ok(Bson::Timestamp(bson::Timestamp {
                        time: t,
                        increment: i,
                    }));
                }
                if m.contains_key("$minKey") {
                    return Ok(Bson::MinKey);
                }
                if m.contains_key("$maxKey") {
                    return Ok(Bson::MaxKey);
                }
            }
            let mut doc = Document::new();
            for (k, val) in m {
                doc.insert(k.clone(), json_to_bson(val)?);
            }
            Ok(Bson::Document(doc))
        }
    }
}

fn parse_i64(s: &str) -> Option<i64> {
    s.parse::<i64>().ok()
}

/// Convert BSON back to plain JSON (ObjectId -> hex string, DateTime -> ISO).
pub fn bson_to_json(b: &Bson) -> Value {
    match b {
        Bson::Double(d) => {
            if d.is_finite() {
                Value::from(*d)
            } else {
                // non-finite doubles have no JSON literal (serde maps them to
                // null) — emit relaxed extended JSON so the value round-trips
                let mut m = serde_json::Map::new();
                m.insert("$numberDouble".into(), Value::from(d.to_string()));
                Value::Object(m)
            }
        }
        Bson::String(s) => Value::from(s.clone()),
        Bson::Array(a) => Value::Array(a.iter().map(bson_to_json).collect()),
        Bson::Document(d) => Value::Object(
            d.iter()
                .map(|(k, v)| (k.clone(), bson_to_json(v)))
                .collect(),
        ),
        Bson::Boolean(x) => Value::from(*x),
        Bson::Null | Bson::Undefined => Value::Null,
        Bson::Int32(i) => Value::from(*i),
        Bson::Int64(i) => Value::from(*i),
        Bson::ObjectId(oid) => Value::from(oid.to_hex()),
        Bson::DateTime(dt) => Value::from((*dt).to_chrono().to_rfc3339()),
        Bson::RegularExpression(re) => {
            let mut m = serde_json::Map::new();
            m.insert("$regex".into(), Value::from(re.pattern.clone()));
            if !re.options.is_empty() {
                m.insert("$options".into(), Value::from(re.options.clone()));
            }
            Value::Object(m)
        }
        Bson::Binary(bin) => {
            let mut m = serde_json::Map::new();
            let mut inner = serde_json::Map::new();
            inner.insert(
                "base64".into(),
                Value::from(base64::engine::general_purpose::STANDARD.encode(&bin.bytes)),
            );
            inner.insert(
                "subType".into(),
                Value::from(format!("{:02x}", bin.subtype)),
            );
            m.insert("$binary".into(), Value::Object(inner));
            Value::Object(m)
        }
        Bson::Timestamp(ts) => {
            let mut m = serde_json::Map::new();
            let mut inner = serde_json::Map::new();
            inner.insert("t".into(), Value::from(ts.time));
            inner.insert("i".into(), Value::from(ts.increment));
            m.insert("$timestamp".into(), Value::Object(inner));
            Value::Object(m)
        }
        Bson::Decimal128(d) => {
            // a plain string would silently change the type on re-insert
            let mut m = serde_json::Map::new();
            m.insert("$numberDecimal".into(), Value::from(d.to_string()));
            Value::Object(m)
        }
        Bson::Symbol(s) => Value::from(s.clone()),
        Bson::JavaScriptCode(s) => Value::from(s.clone()),
        Bson::JavaScriptCodeWithScope(jsc) => {
            let mut m = serde_json::Map::new();
            m.insert("$code".into(), Value::from(jsc.code.clone()));
            m.insert(
                "$scope".into(),
                bson_to_json(&Bson::Document(jsc.scope.clone())),
            );
            Value::Object(m)
        }
        Bson::DbPointer(dbp) => serde_json::to_value(dbp).unwrap_or(Value::Null),
        Bson::MaxKey => Value::Object(
            [("$maxKey".to_string(), Value::from(1))]
                .into_iter()
                .collect(),
        ),
        Bson::MinKey => Value::Object(
            [("$minKey".to_string(), Value::from(1))]
                .into_iter()
                .collect(),
        ),
    }
}

// ---------------------------------------------------------------------------
// Sort parsing
// ---------------------------------------------------------------------------

/// Parse a MongoDB sort spec: {"field": 1, "other": -1}.
pub fn parse_sort(v: &Value) -> Result<Vec<(String, i8)>, ApiError> {
    let mut out = Vec::new();
    let Value::Object(m) = v else {
        return Err(ApiError::new(
            ApiErrorKind::InvalidSort,
            "sort must be a JSON object",
        ));
    };
    if m.is_empty() {
        return Err(ApiError::new(
            ApiErrorKind::InvalidSort,
            "sort must not be empty",
        ));
    }
    for (k, val) in m {
        let dir = match val {
            Value::Number(n) if n.as_i64() == Some(1) => 1,
            Value::Number(n) if n.as_i64() == Some(-1) => -1,
            _ => {
                return Err(ApiError::new(
                    ApiErrorKind::InvalidSort,
                    "sort directions must be 1 or -1",
                ));
            }
        };
        out.push((k.clone(), dir));
    }
    Ok(out)
}

/// Normalize a sort: append `_id` tiebreaker (direction = direction of the
/// last key) so that keyset pagination is deterministic.
pub fn normalize_sort(sort: &[(String, i8)]) -> Vec<(String, i8)> {
    if sort.iter().any(|(f, _)| f == "_id") {
        return sort.to_vec();
    }
    let dir = sort.last().map(|(_, d)| *d).unwrap_or(1);
    let mut out = sort.to_vec();
    out.push(("_id".to_string(), dir));
    out
}

pub fn sort_document(sort: &[(String, i8)]) -> Document {
    let mut d = Document::new();
    for (f, dir) in sort {
        d.insert(f.clone(), if *dir > 0 { 1i32 } else { -1i32 });
    }
    d
}

// ---------------------------------------------------------------------------
// Projection
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub enum ProjectionStyle {
    Include,
    Exclude,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Projection {
    pub style: ProjectionStyle,
    pub fields: std::collections::BTreeSet<String>,
    /// `_id: 0` was requested
    pub exclude_id: bool,
    /// `_id: 1` was requested (include style only)
    pub include_id: bool,
}

/// Parse a client projection spec: `{"a": 1, "b": 0}`. Returns `None` for
/// an empty object (no-op). Values must be 0/1/true/false; mixing inclusion
/// and exclusion is rejected except for `_id`; dotted (`a.b`) and
/// `$`-prefixed (`$meta`, `$slice`, ...) keys are refused — only top-level
/// fields are supported (v1).
pub fn parse_projection(v: &Value) -> Result<Option<Projection>, ApiError> {
    let Value::Object(m) = v else {
        return Err(ApiError::new(
            ApiErrorKind::InvalidProjection,
            "projection must be a JSON object",
        ));
    };
    if m.is_empty() {
        return Ok(None);
    }
    let mut style: Option<ProjectionStyle> = None;
    let mut fields = std::collections::BTreeSet::new();
    let mut exclude_id = false;
    let mut include_id = false;
    for (k, val) in m {
        if k.contains('.') {
            return Err(ApiError::new(
                ApiErrorKind::InvalidProjection,
                "projection of nested fields (dotted keys) is not supported",
            ));
        }
        if k.starts_with('$') {
            return Err(ApiError::new(
                ApiErrorKind::InvalidProjection,
                format!("projection operator {k:?} is not supported"),
            ));
        }
        let incl = match val {
            Value::Bool(b) => *b,
            Value::Number(n) if n.as_i64() == Some(1) => true,
            Value::Number(n) if n.as_i64() == Some(0) => false,
            _ => {
                return Err(ApiError::new(
                    ApiErrorKind::InvalidProjection,
                    "projection values must be 0, 1, true or false",
                ));
            }
        };
        if k == "_id" {
            if incl {
                include_id = true;
            } else {
                exclude_id = true;
            }
            continue; // _id:1 is the default anyway, _id:0 handled at strip time
        }
        match (&style, incl) {
            (None, _) => {
                style = Some(if incl {
                    ProjectionStyle::Include
                } else {
                    ProjectionStyle::Exclude
                })
            }
            (Some(ProjectionStyle::Include), true) | (Some(ProjectionStyle::Exclude), false) => {}
            _ => {
                return Err(ApiError::new(
                    ApiErrorKind::InvalidProjection,
                    "cannot mix inclusion and exclusion in a projection (except _id)",
                ));
            }
        }
        fields.insert(k.clone());
    }
    // Only `_id` keys present: `{_id:1}` keeps the include style (Mongo
    // returns only _id); `{_id:0}` alone means "everything except _id", so it
    // must be an EXCLUDE projection — as include-style it would collapse the
    // output to empty documents.
    let style = match style {
        Some(s) => s,
        None if exclude_id => ProjectionStyle::Exclude,
        None => ProjectionStyle::Include,
    };
    Ok(Some(Projection {
        style,
        fields,
        exclude_id,
        include_id,
    }))
}

/// The projection sent to Mongo: client fields plus everything the keyset
/// machinery needs — every sort field and `_id` — so boundaries and the
/// array-sort guard keep working. Exclude style: sort fields and `_id` are
/// removed from the exclusion set (Mongo keeps returning them); the
/// client-side strip (`projection_strip_fields`) hides them from the output.
/// Returns `None` when no projection is needed at all.
pub fn projection_document(proj: Option<&Projection>, sort: &[(String, i8)]) -> Option<Document> {
    let proj = proj?;
    let mut d = Document::new();
    match proj.style {
        ProjectionStyle::Include => {
            for f in &proj.fields {
                d.insert(f.clone(), 1i32);
            }
            for (f, _) in sort {
                if f != "_id" {
                    d.insert(f.clone(), 1i32);
                }
            }
            d.insert("_id", 1i32);
        }
        ProjectionStyle::Exclude => {
            for f in &proj.fields {
                d.insert(f.clone(), 0i32);
            }
            for (f, _) in sort {
                if f != "_id" {
                    d.remove(f);
                }
            }
            d.remove("_id");
            if d.is_empty() {
                return None;
            }
        }
    }
    Some(d)
}

/// Top-level keys to strip from output documents: in include style, the sort
/// fields + `_id` that were force-added but not requested; in exclude style,
/// the client's exclusions plus `_id` when `_id:0` was requested.
pub fn projection_strip_fields(
    proj: Option<&Projection>,
    sort: &[(String, i8)],
) -> std::collections::BTreeSet<String> {
    let mut strip = std::collections::BTreeSet::new();
    let Some(proj) = proj else {
        return strip;
    };
    match proj.style {
        ProjectionStyle::Include => {
            for (f, _) in sort {
                if f != "_id" && !proj.fields.contains(f) {
                    strip.insert(f.clone());
                }
            }
            if !proj.include_id {
                strip.insert("_id".into());
            }
        }
        ProjectionStyle::Exclude => {
            strip = proj.fields.clone();
            if proj.exclude_id {
                strip.insert("_id".into());
            }
        }
    }
    strip
}

/// Serialize a document to JSON, skipping the given top-level keys (the
/// projection strip). Values are encoded by `bson_to_json` unchanged.
pub fn bson_to_json_projected(doc: &Document, strip: &std::collections::BTreeSet<String>) -> Value {
    let mut m = serde_json::Map::new();
    for (k, v) in doc {
        if strip.contains(k) {
            continue;
        }
        m.insert(k.clone(), bson_to_json(v));
    }
    Value::Object(m)
}

// ---------------------------------------------------------------------------
// Cursor
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Cursor {
    pub v: u32,
    pub id: String,
    pub db: String,
    pub coll: String,
    pub sort: Vec<(String, i8)>,
    /// last seen values for each sort field (BSON, stored as relaxed extjson string)
    pub last: Vec<String>,
}

fn bson_to_cursor_json(b: &Bson) -> Result<String, String> {
    // canonical extended JSON round-trips every BSON type exactly
    serde_json::to_string(&b.clone().into_canonical_extjson()).map_err(|e| e.to_string())
}

fn cursor_json_to_bson(s: &str) -> Result<Bson, String> {
    let v: Value = serde_json::from_str(s).map_err(|e| format!("bad cursor value: {e}"))?;
    Bson::try_from(v).map_err(|e| format!("bad cursor value: {e}"))
}

impl Cursor {
    pub fn encode(&self) -> String {
        let json = serde_json::to_string(self).unwrap_or_default();
        URL_SAFE_NO_PAD.encode(json.as_bytes())
    }

    pub fn decode(s: &str) -> Result<Cursor, ApiError> {
        let bytes = URL_SAFE_NO_PAD
            .decode(s.as_bytes())
            .map_err(|_| ApiError::new(ApiErrorKind::InvalidCursor, "malformed cursor"))?;
        let cur: Cursor = serde_json::from_slice(&bytes)
            .map_err(|_| ApiError::new(ApiErrorKind::InvalidCursor, "malformed cursor"))?;
        if cur.v != 1 || cur.sort.is_empty() || cur.last.len() != cur.sort.len() {
            return Err(ApiError::new(ApiErrorKind::InvalidCursor, "corrupt cursor"));
        }
        Ok(cur)
    }

    pub fn last_bson(&self) -> Result<Vec<Bson>, ApiError> {
        self.last
            .iter()
            .map(|s| {
                cursor_json_to_bson(s).map_err(|e| ApiError::new(ApiErrorKind::InvalidCursor, e))
            })
            .collect()
    }
}

/// BSON type brackets in MongoDB's sort order, as measured on MongoDB 8
/// (MinKey < Null < NaN < Numbers < Symbol < String < Object < Array <
/// BinData < ObjectId < Bool < Date < Timestamp < Regex < JS < MaxKey;
/// NaN sorts before -Inf ascending). Query operators only compare values of
/// the SAME type (except numbers), but the sort orders by type bracket — so a
/// keyset boundary at a null/missing value, NaN, or at a type transition
/// needs explicit fallback branches, or the rest of the data is silently
/// dropped.
const TYPE_ORDER: &[&str] = &[
    "minKey",
    "null",
    "nan",
    "number",
    "symbol",
    "string",
    "object",
    "array",
    "binData",
    "objectId",
    "dbPointer",
    "bool",
    "date",
    "timestamp",
    "regex",
    "javascript",
    "javascriptWithScope",
    "maxKey",
];

fn bson_type_bracket(b: &Bson) -> &'static str {
    match b {
        Bson::Double(d) if d.is_nan() => "nan",
        Bson::Double(_) | Bson::Int32(_) | Bson::Int64(_) | Bson::Decimal128(_) => "number",
        Bson::String(_) => "string",
        Bson::Array(_) => "array",
        Bson::Document(_) => "object",
        Bson::Boolean(_) => "bool",
        Bson::Null | Bson::Undefined => "null",
        Bson::ObjectId(_) => "objectId",
        Bson::DateTime(_) => "date",
        Bson::RegularExpression(_) => "regex",
        Bson::Binary(_) => "binData",
        Bson::Timestamp(_) => "timestamp",
        Bson::Symbol(_) => "symbol",
        Bson::JavaScriptCode(_) => "javascript",
        Bson::JavaScriptCodeWithScope(_) => "javascriptWithScope",
        Bson::MinKey => "minKey",
        Bson::MaxKey => "maxKey",
        Bson::DbPointer(_) => "dbPointer",
    }
}

/// `$type` names in brackets strictly after (ascending) the value's bracket.
fn type_brackets_after(b: &Bson) -> Vec<&'static str> {
    let t = bson_type_bracket(b);
    let idx = TYPE_ORDER.iter().position(|x| *x == t).unwrap_or(0);
    TYPE_ORDER[idx + 1..].to_vec()
}

/// `$type` names in brackets strictly before (descending) the value's bracket.
fn type_brackets_before(b: &Bson) -> Vec<&'static str> {
    let t = bson_type_bracket(b);
    let idx = TYPE_ORDER.iter().position(|x| *x == t).unwrap_or(0);
    TYPE_ORDER[..idx].to_vec()
}

/// Build the keyset continuation condition from a cursor.
/// Standard multi-column keyset: for sort keys (f0..fn) with last values
/// (v0..vn): $or of {f0 > v0}, {f0 = v0, f1 > v1}, ..., {f0=v0..f(n-1)=v(n-1), fn > vn}.
/// Each column also gets a type-bracket fallback branch ($type) because query
/// operators only match same-type values while the sort orders by BSON type.
pub fn keyset_condition(sort: &[(String, i8)], last: &[Bson]) -> Document {
    let mut ors: Vec<Document> = Vec::new();
    for k in 0..sort.len() {
        // --- same-type continuation ---
        // skipped for a null boundary: {$gt: null} matches ALL null/missing
        // values and would re-serve already-paged documents (the _id
        // tiebreaker branch below handles the rest)
        let is_null = matches!(last[k], Bson::Null | Bson::Undefined);
        let is_nan = matches!(last[k], Bson::Double(d) if d.is_nan());
        if !is_null {
            let mut part = Document::new();
            for j in 0..k {
                part.insert(sort[j].0.clone(), last[j].clone());
            }
            if is_nan {
                // NaN comparisons never match, and NaN sorts before -Inf
                // ascending: the numeric continuation starts at -Inf and must
                // INCLUDE the -Inf tie-group itself ($gte, not $gt)
                if sort[k].1 > 0 {
                    let mut cmp = Document::new();
                    cmp.insert("$gte", Bson::Double(f64::NEG_INFINITY));
                    part.insert(sort[k].0.clone(), Bson::Document(cmp));
                    ors.push(part);
                }
                // descending: nothing sorts below the NaN group except the
                // minKey/null brackets (type branches below)
            } else {
                let op = if sort[k].1 > 0 { "$gt" } else { "$lt" };
                let mut cmp = Document::new();
                cmp.insert(op, last[k].clone());
                part.insert(sort[k].0.clone(), Bson::Document(cmp));
                ors.push(part);
            }
        }
        // --- type-bracket continuation: values whose BSON type sorts after
        // (asc) or before (desc) the boundary value's type ---
        let mut brackets = if sort[k].1 > 0 {
            type_brackets_after(&last[k])
        } else {
            type_brackets_before(&last[k])
        };
        if is_nan && sort[k].1 > 0 {
            // the numeric bracket is already covered by the $gt: -Inf branch;
            // $type: "number" would match NaN itself and re-serve the group
            brackets.retain(|s| *s != "number");
        }
        if !brackets.is_empty() {
            // "null" and "nan" have no $type form that covers missing fields
            // / NaN: match them with equality branches ({f: null} matches
            // explicit nulls AND missing fields; {f: NaN} matches NaN docs)
            let has_null = brackets.iter().any(|s| *s == "null");
            let has_nan = brackets.iter().any(|s| *s == "nan");
            let rest: Vec<&str> = brackets
                .into_iter()
                .filter(|s| *s != "null" && *s != "nan")
                .collect();
            if !rest.is_empty() {
                let mut part = Document::new();
                for j in 0..k {
                    part.insert(sort[j].0.clone(), last[j].clone());
                }
                let mut tc = Document::new();
                tc.insert(
                    "$type",
                    Bson::Array(
                        rest.into_iter()
                            .map(|s| Bson::String(s.to_string()))
                            .collect(),
                    ),
                );
                part.insert(sort[k].0.clone(), Bson::Document(tc));
                ors.push(part);
            }
            if has_null {
                let mut part = Document::new();
                for j in 0..k {
                    part.insert(sort[j].0.clone(), last[j].clone());
                }
                part.insert(sort[k].0.clone(), Bson::Null);
                ors.push(part);
            }
            if has_nan {
                let mut part = Document::new();
                for j in 0..k {
                    part.insert(sort[j].0.clone(), last[j].clone());
                }
                part.insert(sort[k].0.clone(), Bson::Double(f64::NAN));
                ors.push(part);
            }
        }
    }
    doc! { "$or": Bson::Array(ors.into_iter().map(Bson::Document).collect()) }
}

/// Build the full filter: user filter AND keyset condition (when cursor given).
pub fn build_filter(user: Option<Document>, cursor: Option<&Cursor>) -> Result<Document, ApiError> {
    match (user, cursor) {
        (Some(u), Some(c)) => {
            let kc = keyset_condition(&c.sort, &c.last_bson()?);
            Ok(doc! { "$and": Bson::Array(vec![Bson::Document(u), Bson::Document(kc)]) })
        }
        (Some(u), None) => Ok(u),
        (None, Some(c)) => Ok(keyset_condition(&c.sort, &c.last_bson()?)),
        (None, None) => Ok(Document::new()),
    }
}

// ---------------------------------------------------------------------------
// Query execution
// ---------------------------------------------------------------------------

/// Run a paginated find. Returns (docs, has_more, next_cursor_values).
pub async fn find_page(
    state: &AppState,
    db: &str,
    coll: &str,
    filter: Document,
    sort: &[(String, i8)],
    limit: u32,
    projection: Option<Document>,
    _user_cursor: Option<&Cursor>,
) -> Result<(Vec<Document>, bool, Option<Document>), ApiError> {
    let collection = state.mongo.database(db).collection::<Document>(coll);
    let mut find = collection
        .find(filter)
        .sort(sort_document(sort))
        .limit(limit as i64 + 1);
    if let Some(p) = projection {
        find = find.projection(p);
    }
    let mut cursor = find.await?;
    let mut docs = Vec::with_capacity(limit as usize + 1);
    while let Some(d) = cursor.try_next().await? {
        docs.push(d);
        if docs.len() as u32 > limit {
            break;
        }
    }
    let has_more = docs.len() as u32 > limit;
    if has_more {
        docs.truncate(limit as usize);
    }
    let last = docs.last().cloned();
    Ok((docs, has_more, last))
}

/// Build a next-cursor from the last document of a page.
pub fn make_next_cursor(
    state: &AppState,
    db: &str,
    coll: &str,
    sort: &[(String, i8)],
    last_doc: &Document,
) -> Result<Cursor, ApiError> {
    let mut values = Vec::with_capacity(sort.len());
    for (f, _) in sort {
        // Mongo sorts missing fields as null; a page-boundary doc may lack the
        // sort key, so encode Bson::Null instead of failing with a 500.
        let val = last_doc.get(f).cloned().unwrap_or(Bson::Null);
        values.push(bson_to_cursor_json(&val).map_err(|e| ApiError::internal(e))?);
    }
    let seq = state
        .cursor_seq
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let id = format!("c{:x}", seq);
    let now = crate::state::now_ms();
    state.cursors.insert(
        id.clone(),
        crate::state::CursorInfo {
            id: id.clone(),
            db: db.to_string(),
            coll: coll.to_string(),
            created_ms: now,
            last_used_ms: std::sync::atomic::AtomicI64::new(now),
            uses: std::sync::atomic::AtomicU64::new(0),
        },
    );
    Ok(Cursor {
        v: 1,
        id,
        db: db.to_string(),
        coll: coll.to_string(),
        sort: sort.to_vec(),
        last: values,
    })
}

/// Touch the cursor registry (used when a page request arrives with a cursor).
pub fn touch_cursor(state: &AppState, id: &str) {
    if let Some(c) = state.cursors.get(id) {
        c.last_used_ms
            .store(crate::state::now_ms(), std::sync::atomic::Ordering::Relaxed);
        c.uses.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }
}

/// Keep the registry bounded.
pub fn cursor_sweep(state: &AppState) {
    if state.cursors.len() <= 2000 {
        return;
    }
    let mut oldest: Vec<(String, i64)> = state
        .cursors
        .iter()
        .map(|c| {
            (
                c.id.clone(),
                c.last_used_ms.load(std::sync::atomic::Ordering::Relaxed),
            )
        })
        .collect();
    oldest.sort_by_key(|(_, t)| *t);
    let excess = oldest.len().saturating_sub(1000);
    for (id, _) in oldest.into_iter().take(excess) {
        state.cursors.remove(&id);
    }
}

// ---------------------------------------------------------------------------
// CRUD
// ---------------------------------------------------------------------------

/// extract and convert a body field; returns None when absent, error when present-but-invalid
pub fn require_object<'a>(bs: &'a Bson, what: &str) -> Result<&'a Document, ApiError> {
    match bs {
        Bson::Document(d) => Ok(d),
        _ => Err(ApiError::bad_request(format!(
            "{what} must be a JSON object"
        ))),
    }
}

// ---------------------------------------------------------------------------
// Listing
// ---------------------------------------------------------------------------

pub async fn list_databases(state: &AppState) -> Result<Vec<String>, ApiError> {
    let names = state.mongo.list_database_names().await?;
    Ok(names
        .into_iter()
        .filter(|n| n != "admin" && n != "local" && n != "config")
        .collect())
}

pub async fn list_collections(state: &AppState, db: &str) -> Result<Vec<String>, ApiError> {
    let names = state.mongo.database(db).list_collection_names().await?;
    Ok(names)
}

// ---------------------------------------------------------------------------
// generic CRUD wrappers (used by routes)
// ---------------------------------------------------------------------------

pub async fn insert_one(
    state: &AppState,
    db: &str,
    coll: &str,
    doc: Document,
) -> Result<Bson, ApiError> {
    let r = state
        .mongo
        .database(db)
        .collection::<Document>(coll)
        .insert_one(doc)
        .await?;
    // the real stored _id — for non-ObjectId ids, ObjectId::default() would
    // fabricate a random id that cannot locate the inserted document
    Ok(r.inserted_id)
}

pub async fn insert_many(
    state: &AppState,
    db: &str,
    coll: &str,
    docs: Vec<Document>,
) -> Result<Vec<Bson>, ApiError> {
    let r = state
        .mongo
        .database(db)
        .collection::<Document>(coll)
        .insert_many(docs)
        .await?;
    // inserted_ids is keyed by input index (a HashMap in driver 3.x) — sort
    // by index so the response order matches the request order
    let mut ids: Vec<(usize, Bson)> = r.inserted_ids.into_iter().collect();
    ids.sort_by_key(|(i, _)| *i);
    Ok(ids.into_iter().map(|(_, id)| id).collect())
}

pub async fn update_many(
    state: &AppState,
    db: &str,
    coll: &str,
    filter: Document,
    update: Document,
    upsert: bool,
) -> Result<mongodb::results::UpdateResult, ApiError> {
    state
        .mongo
        .database(db)
        .collection::<Document>(coll)
        .update_many(filter, update)
        .upsert(upsert)
        .await
        .map_err(Into::into)
}

pub async fn delete_many(
    state: &AppState,
    db: &str,
    coll: &str,
    filter: Document,
) -> Result<mongodb::results::DeleteResult, ApiError> {
    state
        .mongo
        .database(db)
        .collection::<Document>(coll)
        .delete_many(filter)
        .await
        .map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bson_roundtrip() {
        let v: Value = serde_json::from_str(
            r#"{"a":1,"b":"x","c":[true,null],"d":{"e":2.5},"_id":{"$oid":"665f00000000000000000000"}}"#,
        )
        .unwrap();
        let b = json_to_bson(&v).unwrap();
        let j = bson_to_json(&b);
        assert_eq!(j["a"], 1);
        assert_eq!(j["b"], "x");
        assert_eq!(j["_id"], "665f00000000000000000000");
        assert_eq!(j["d"]["e"], 2.5);
    }

    #[test]
    fn extjson_tokens() {
        let v: Value = serde_json::from_str(
            r#"{"n":{"$numberLong":"42"},"d":{"$date":"2024-01-01T00:00:00Z"}}"#,
        )
        .unwrap();
        let b = json_to_bson(&v).unwrap();
        match &b {
            Bson::Document(d) => {
                assert!(matches!(d.get("n"), Some(Bson::Int64(42))));
                assert!(matches!(d.get("d"), Some(Bson::DateTime(_))));
            }
            _ => panic!(),
        }
    }

    #[test]
    fn sort_parse_and_normalize() {
        let v: Value = serde_json::from_str(r#"{"age": -1, "name": 1}"#).unwrap();
        let s = parse_sort(&v).unwrap();
        assert_eq!(s, vec![("age".into(), -1), ("name".into(), 1)]);
        let n = normalize_sort(&s);
        assert_eq!(n[2], ("_id".into(), 1));
        // _id already present -> unchanged
        let v2: Value = serde_json::from_str(r#"{"_id": 1}"#).unwrap();
        let s2 = parse_sort(&v2).unwrap();
        assert_eq!(normalize_sort(&s2).len(), 1);
    }

    #[test]
    fn keyset_condition_shape() {
        let sort = vec![("a".into(), 1i8), ("b".into(), -1i8), ("_id".into(), -1i8)];
        let last = vec![
            Bson::Int32(1),
            Bson::String("x".into()),
            Bson::ObjectId(ObjectId::new()),
        ];
        let cond = keyset_condition(&sort, &last);
        let ors = cond.get_array("$or").unwrap();
        // a: same-type + $type; b: same + $type + {b: null} + {b: NaN};
        // _id: same + $type + {_id: null} + {_id: NaN}
        assert_eq!(ors.len(), 10);
        // first branch: {"a": {"$gt": 1}}
        assert!(
            ors[0]
                .as_document()
                .unwrap()
                .get("a")
                .unwrap()
                .as_document()
                .unwrap()
                .contains_key("$gt")
        );
        // second branch: {"a": {"$type": […]}} — every bracket after "number"
        let t = ors[1]
            .as_document()
            .unwrap()
            .get("a")
            .unwrap()
            .as_document()
            .unwrap();
        let types = t.get_array("$type").unwrap();
        assert_eq!(types.len(), 14);
        assert_eq!(types[0], Bson::String("symbol".into()));
        // column b (desc): {"a": 1, "b": {"$lt": "x"}} then the $type branch
        let b3 = ors[2].as_document().unwrap();
        assert_eq!(b3.get("a").unwrap(), &Bson::Int32(1));
        assert!(
            b3.get("b")
                .unwrap()
                .as_document()
                .unwrap()
                .contains_key("$lt")
        );
        let t4 = ors[3]
            .as_document()
            .unwrap()
            .get("b")
            .unwrap()
            .as_document()
            .unwrap();
        let types4 = t4.get_array("$type").unwrap();
        // brackets before "string" minus null: minKey, number, symbol
        assert_eq!(types4.len(), 3);
        assert_eq!(types4.last().unwrap(), &Bson::String("symbol".into()));
        // the null bracket is a separate {b: null} branch (matches missing)
        assert!(matches!(
            ors[4].as_document().unwrap().get("b"),
            Some(bson::Bson::Null)
        ));
    }

    #[test]
    fn keyset_desc_boundary_matches_null_and_missing() {
        // descending over a numeric boundary: the null bracket (explicit null
        // AND missing fields) comes entirely after it and must be reachable
        let sort = vec![("f".into(), -1i8), ("_id".into(), -1i8)];
        let last = vec![Bson::Int32(5), Bson::ObjectId(ObjectId::new())];
        let cond = keyset_condition(&sort, &last);
        let ors = cond.get_array("$or").unwrap();
        // {$lt:5}, $type[minKey], {f:null}, {f:NaN}, _id $lt, _id $type, _id null, _id NaN
        assert_eq!(ors.len(), 8);
        assert!(
            ors.iter()
                .any(|o| o.as_document().unwrap().get("f") == Some(&Bson::Null))
        );
        assert!(ors.iter().any(|o| {
            matches!(
                o.as_document().unwrap().get("f"),
                Some(Bson::Double(d)) if d.is_nan()
            )
        }));
    }

    #[test]
    fn keyset_null_boundary_skips_gt_null() {
        let sort = vec![("f".into(), 1i8), ("_id".into(), 1i8)];
        let last = vec![Bson::Null, Bson::ObjectId(ObjectId::new())];
        let cond = keyset_condition(&sort, &last);
        let ors = cond.get_array("$or").unwrap();
        // no {$gt: null} branch (it would re-serve every null/missing doc);
        // f gets $type + the {f: NaN} branch, _id gets $gt tiebreaker + $type
        assert_eq!(ors.len(), 4);
        assert!(
            ors[0]
                .as_document()
                .unwrap()
                .get("f")
                .unwrap()
                .as_document()
                .unwrap()
                .contains_key("$type")
        );
        assert!(matches!(
            ors[1].as_document().unwrap().get("f"),
            Some(Bson::Double(d)) if d.is_nan()
        ));
        let b1 = ors[2].as_document().unwrap();
        assert_eq!(b1.get("f").unwrap(), &Bson::Null);
        assert!(
            b1.get("_id")
                .unwrap()
                .as_document()
                .unwrap()
                .contains_key("$gt")
        );
    }

    #[test]
    fn keyset_nan_boundary() {
        // asc NaN boundary: the numeric continuation is $gt: -Inf (NaN
        // comparisons never match), and $type must not include "number"
        // (it would re-serve the NaN group)
        let sort = vec![("f".into(), 1i8), ("_id".into(), 1i8)];
        let last = vec![Bson::Double(f64::NAN), Bson::ObjectId(ObjectId::new())];
        let cond = keyset_condition(&sort, &last);
        let ors = cond.get_array("$or").unwrap();
        let f0 = ors[0]
            .as_document()
            .unwrap()
            .get("f")
            .unwrap()
            .as_document()
            .unwrap();
        assert_eq!(f0.get("$gte"), Some(&Bson::Double(f64::NEG_INFINITY)));
        for o in ors {
            let d = o.as_document().unwrap();
            if let Some(t) = d.get("f").and_then(|v| v.as_document()) {
                if let Ok(types) = t.get_array("$type") {
                    assert!(
                        !types.contains(&Bson::String("number".into())),
                        "$type must not include number after a NaN boundary"
                    );
                }
            }
        }
        // desc NaN boundary: no $lt branch (matches nothing anyway), the
        // null bracket (incl. missing) is reachable via {f: null}
        let sort = vec![("f".into(), -1i8), ("_id".into(), -1i8)];
        let last = vec![Bson::Double(f64::NAN), Bson::ObjectId(ObjectId::new())];
        let cond = keyset_condition(&sort, &last);
        let ors = cond.get_array("$or").unwrap();
        assert!(ors.iter().all(|o| {
            o.as_document()
                .unwrap()
                .get("f")
                .and_then(|v| v.as_document())
                .map(|c| !c.contains_key("$lt"))
                .unwrap_or(true)
        }));
        assert!(
            ors.iter()
                .any(|o| o.as_document().unwrap().get("f") == Some(&Bson::Null))
        );
        assert!(ors.iter().any(|o| {
            matches!(
                o.as_document().unwrap().get("f"),
                Some(Bson::Double(d)) if d.is_nan()
            )
        }));
    }

    #[test]
    fn cursor_roundtrip() {
        let c = Cursor {
            v: 1,
            id: "c1".into(),
            db: "db1".into(),
            coll: "coll".into(),
            sort: vec![("a".into(), 1i8), ("_id".into(), 1i8)],
            last: vec![
                serde_json::to_string(&Bson::Int32(1).into_canonical_extjson()).unwrap(),
                serde_json::to_string(&Bson::ObjectId(ObjectId::new()).into_canonical_extjson())
                    .unwrap(),
            ],
        };
        let enc = c.encode();
        let dec = Cursor::decode(&enc).unwrap();
        assert_eq!(dec.db, c.db);
        assert_eq!(dec.sort, c.sort);
        assert_eq!(dec.last, c.last);
        let vals = dec.last_bson().unwrap();
        assert!(matches!(vals[0], Bson::Int32(1)));
        assert!(matches!(vals[1], Bson::ObjectId(_)));
        assert!(Cursor::decode("garbage!!").is_err());
    }

    #[test]
    fn cursor_rejects_tampering() {
        // valid envelope, wrong field count
        let c = Cursor {
            v: 1,
            id: "x".into(),
            db: "d".into(),
            coll: "c".into(),
            sort: vec![("a".into(), 1i8)],
            last: vec![],
        };
        assert!(Cursor::decode(&c.encode()).is_err());
    }

    #[test]
    fn build_filter_rejects_bad_cursor_values() {
        // JSON parses but BSON conversion fails (non-finite f64): build_filter
        // must return an error, not panic on an empty last slice
        let c = Cursor {
            v: 1,
            id: "x".into(),
            db: "db1".into(),
            coll: "rev".into(),
            sort: vec![("a".into(), 1i8)],
            last: vec!["1e999".into()],
        };
        assert!(build_filter(None, Some(&c)).is_err());
        assert!(build_filter(Some(Document::new()), Some(&c)).is_err());
    }

    #[test]
    fn u64_above_i64_max_becomes_decimal128() {
        let v: Value = serde_json::from_str(r#"{"n":18446744073709551615}"#).unwrap();
        let b = json_to_bson(&v).unwrap();
        match &b {
            Bson::Document(d) => assert!(matches!(d.get("n"), Some(Bson::Decimal128(_)))),
            _ => panic!("expected document"),
        }
        // and it round-trips as relaxed extended JSON
        let j = bson_to_json(&b);
        assert_eq!(
            j,
            serde_json::json!({"n": {"$numberDecimal": "18446744073709551615"}})
        );
    }

    #[test]
    fn regex_two_key_form() {
        let v: Value = serde_json::from_str(r#"{"r":{"$regex":"^ab","$options":"i"}}"#).unwrap();
        let b = json_to_bson(&v).unwrap();
        match &b {
            Bson::Document(d) => match d.get("r") {
                Some(Bson::RegularExpression(re)) => {
                    assert_eq!(re.pattern, "^ab");
                    assert_eq!(re.options, "i");
                }
                _ => panic!("not a regex"),
            },
            _ => panic!("expected document"),
        }
    }

    #[test]
    fn timestamp_validation() {
        let ok: Value = serde_json::from_str(r#"{"t":{"$timestamp":{"t":1,"i":2}}}"#).unwrap();
        assert!(json_to_bson(&ok).is_ok());
        let nonobj: Value = serde_json::from_str(r#"{"t":{"$timestamp":"x"}}"#).unwrap();
        assert!(json_to_bson(&nonobj).is_err());
        let neg: Value = serde_json::from_str(r#"{"t":{"$timestamp":{"t":-1,"i":0}}}"#).unwrap();
        assert!(json_to_bson(&neg).is_err());
        // values >= 2^32 must not silently truncate
        let big: Value =
            serde_json::from_str(r#"{"t":{"$timestamp":{"t":4294967296,"i":0}}}"#).unwrap();
        assert!(json_to_bson(&big).is_err());
    }

    #[test]
    fn non_finite_double_roundtrip() {
        let v: Value = serde_json::from_str(r#"{"$numberDouble":"NaN"}"#).unwrap();
        let b = json_to_bson(&v).unwrap();
        assert!(matches!(b, Bson::Double(d) if d.is_nan()));
        let j = bson_to_json(&b);
        assert_eq!(j, serde_json::json!({"$numberDouble": "NaN"}));
    }

    #[test]
    fn decimal128_roundtrip() {
        let v: Value = serde_json::from_str(r#"{"$numberDecimal":"12345.6789"}"#).unwrap();
        let b = json_to_bson(&v).unwrap();
        assert!(matches!(b, Bson::Decimal128(_)));
        let j = bson_to_json(&b);
        assert_eq!(j, serde_json::json!({"$numberDecimal": "12345.6789"}));
    }

    #[test]
    fn parse_projection_rules() {
        use std::collections::BTreeSet;
        let parse = |s: &str| {
            let v: Value = serde_json::from_str(s).unwrap();
            parse_projection(&v)
        };
        // valid include / exclude / boolean values / _id:0 alone
        let inc = parse(r#"{"a":1,"b":true}"#).unwrap().unwrap();
        assert_eq!(inc.style, ProjectionStyle::Include);
        assert_eq!(inc.fields, BTreeSet::from(["a".into(), "b".into()]));
        assert!(!inc.exclude_id && !inc.include_id);
        let exc = parse(r#"{"a":0,"c":false}"#).unwrap().unwrap();
        assert_eq!(exc.style, ProjectionStyle::Exclude);
        assert_eq!(exc.fields, BTreeSet::from(["a".into(), "c".into()]));
        let id0 = parse(r#"{"_id":0}"#).unwrap().unwrap();
        assert!(id0.exclude_id && id0.fields.is_empty());
        assert_eq!(id0.style, ProjectionStyle::Exclude); // everything except _id
        let id1 = parse(r#"{"a":1,"_id":1}"#).unwrap().unwrap();
        assert!(id1.include_id && !id1.exclude_id);
        // empty object is a no-op
        assert!(parse(r#"{}"#).unwrap().is_none());
        // rejections
        assert!(parse(r#"[1,2]"#).is_err());
        assert!(parse(r#""a""#).is_err());
        assert!(parse(r#"{"a":1,"b":0}"#).is_err()); // mixed
        assert!(parse(r#"{"a":2}"#).is_err());
        assert!(parse(r#"{"a":"1"}"#).is_err());
        assert!(parse(r#"{"a":null}"#).is_err());
        assert!(parse(r#"{"a":{}}"#).is_err());
        assert!(parse(r#"{"a.b":1}"#).is_err()); // dotted
        assert!(parse(r#"{"$meta":"textScore"}"#).is_err()); // operator key
        assert!(parse(r#"{"a":{"$slice":2}}"#).is_err());
        // _id may appear in either style without mixing errors
        assert!(parse(r#"{"a":0,"_id":1}"#).unwrap().is_some());
    }

    #[test]
    fn projection_union_and_strip() {
        use std::collections::BTreeSet;
        let parse = |s: &str| {
            let v: Value = serde_json::from_str(s).unwrap();
            parse_projection(&v).unwrap()
        };
        let sort: Vec<(String, i8)> = vec![("a".into(), 1), ("_id".into(), 1)];

        // include {b}: Mongo sees b + sort field a + _id; strip hides a and _id
        let p = parse(r#"{"b":1}"#);
        assert_eq!(
            projection_document(p.as_ref(), &sort).unwrap(),
            doc! { "b": 1i32, "a": 1i32, "_id": 1i32 }
        );
        assert_eq!(
            projection_strip_fields(p.as_ref(), &sort),
            BTreeSet::from(["a".into(), "_id".into()])
        );

        // include {a, _id}: nothing stripped
        let p = parse(r#"{"a":1,"_id":1}"#);
        assert_eq!(projection_strip_fields(p.as_ref(), &sort), BTreeSet::new());

        // exclude {b} where b is NOT a sort field: Mongo doc keeps {b:0}; strip {b}
        let p = parse(r#"{"b":0}"#);
        assert_eq!(
            projection_document(p.as_ref(), &sort).unwrap(),
            doc! { "b": 0i32 }
        );
        assert_eq!(
            projection_strip_fields(p.as_ref(), &sort),
            BTreeSet::from(["b".into()])
        );

        // exclude {a, _id:0} where a IS the sort field: a and _id are removed
        // from the Mongo exclusion so the keyset keeps working (collapses to
        // None — Mongo gets no projection); both stripped from client output
        let p = parse(r#"{"a":0,"_id":0}"#);
        assert!(projection_document(p.as_ref(), &sort).is_none());
        assert_eq!(
            projection_strip_fields(p.as_ref(), &sort),
            BTreeSet::from(["a".into(), "_id".into()])
        );

        // no projection at all
        assert!(projection_document(None, &sort).is_none());
        assert!(projection_strip_fields(None, &sort).is_empty());
    }

    #[test]
    fn projected_serialization() {
        use std::collections::BTreeSet;
        let doc = doc! {
            "a": 1i32,
            "b": "x",
            "nested": { "deep": 2i32, "keep": true },
            "_id": ObjectId::from_bytes([0; 12]),
        };
        // empty strip == full bson_to_json output
        let full = bson_to_json_projected(&doc, &BTreeSet::new());
        assert_eq!(full, bson_to_json(&Bson::Document(doc.clone())));
        // strip top-level keys; nested content untouched
        let out = bson_to_json_projected(&doc, &BTreeSet::from(["a".into(), "_id".into()]));
        let o = out.as_object().unwrap();
        assert!(!o.contains_key("a") && !o.contains_key("_id"));
        assert_eq!(o["b"], "x");
        assert_eq!(o["nested"], serde_json::json!({ "deep": 2, "keep": true }));
    }

    /// Simulate Mongo's server-side projection over a doc, then apply the
    /// client strip — the visible fields must be exactly source ∩ expected.
    #[test]
    fn projection_pipeline_visible_fields() {
        use std::collections::BTreeSet;
        let parse = |s: &str| {
            let v: Value = serde_json::from_str(s).unwrap();
            parse_projection(&v).unwrap()
        };
        let sort: Vec<(String, i8)> = vec![("a".into(), 1), ("_id".into(), 1)];
        let source = doc! { "a": 1i32, "b": "x", "c": 3.5, "extra": true, "_id": ObjectId::from_bytes([0; 12]) };

        let cases: Vec<(&str, BTreeSet<String>)> = vec![
            // include {b}: visible = {b} (sort field a + _id hidden)
            (r#"{"b":1}"#, BTreeSet::from(["b".into()])),
            // include {b, _id:1}: visible = {b, _id}
            (
                r#"{"b":1,"_id":1}"#,
                BTreeSet::from(["b".into(), "_id".into()]),
            ),
            // include {_id:0} alone: exclude-style -> everything except _id
            (
                r#"{"_id":0}"#,
                BTreeSet::from(["a".into(), "b".into(), "c".into(), "extra".into()]),
            ),
            // exclude {c, _id:0}: visible = {a, b, extra}
            (
                r#"{"c":0,"_id":0}"#,
                BTreeSet::from(["a".into(), "b".into(), "extra".into()]),
            ),
            // exclude {b} (non-sort field): visible = {a, c, extra, _id}
            (
                r#"{"b":0}"#,
                BTreeSet::from(["a".into(), "c".into(), "extra".into(), "_id".into()]),
            ),
            // no projection: everything
            (
                "{}",
                BTreeSet::from([
                    "a".into(),
                    "b".into(),
                    "c".into(),
                    "extra".into(),
                    "_id".into(),
                ]),
            ),
        ];
        for (spec, expected) in cases {
            let proj = parse(spec);
            let mongo = projection_document(proj.as_ref(), &sort);
            // apply the Mongo projection like the server would
            let mut sim = Document::new();
            match &mongo {
                None => sim = source.clone(),
                Some(m) => {
                    let is_exclude = m.values().any(|v| matches!(v, Bson::Int32(0)));
                    if is_exclude {
                        for (k, v) in &source {
                            if !m.contains_key(k) {
                                sim.insert(k.clone(), v.clone());
                            }
                        }
                    } else {
                        for (k, v) in &source {
                            if m.contains_key(k) {
                                sim.insert(k.clone(), v.clone());
                            }
                        }
                    }
                }
            }
            let strip = projection_strip_fields(proj.as_ref(), &sort);
            let visible = bson_to_json_projected(&sim, &strip);
            let visible_keys: BTreeSet<String> =
                visible.as_object().unwrap().keys().cloned().collect();
            assert_eq!(visible_keys, expected, "spec {spec}: visible keys mismatch");
            // and values survived (spot-check b)
            if spec == r#"{"b":1}"# {
                assert_eq!(visible["b"], "x");
            }
        }
    }

    /// Integration test: keyset pagination over mixed-type / missing-field
    /// data must return EXACTLY the same _id sequence as a single full scan,
    /// for both directions and several limits. Runs only when
    /// XDB_TEST_MONGO_URI is set (uses a scratch db, dropped afterwards).
    #[test]
    fn keyset_pagination_equivalence() {
        let Ok(uri) = std::env::var("XDB_TEST_MONGO_URI") else {
            eprintln!("skipping keyset_pagination_equivalence (XDB_TEST_MONGO_URI unset)");
            return;
        };
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async move {
            let client = mongodb::Client::with_uri_str(&uri).await.unwrap();
            let db = client.database("xdb_test");
            let coll_name = format!("page_eq_{}", std::process::id());
            let coll = db.collection::<Document>(&coll_name);
            coll.drop().await.unwrap();

            // deterministic mixed-type dataset: fields a/b/c drawn from null,
            // MISSING, int, double, string, bool, date
            let mut state: u64 = 0x5eed_cafe;
            let mut rnd = move || {
                state = state
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(1442695040888963407);
                (state >> 33) as u32
            };
            let mut docs = Vec::new();
            for i in 0..50u32 {
                let mut d = Document::new();
                for f in ["a", "b", "c"] {
                    match rnd() % 10 {
                        0 => {
                            d.insert(f, Bson::Null);
                        }
                        1 => {} // missing
                        2 => {
                            d.insert(f, Bson::Int32((rnd() % 200) as i32 - 100));
                        }
                        3 => {
                            d.insert(f, Bson::Double((rnd() % 1000) as f64 / 10.0));
                        }
                        4 => {
                            d.insert(f, Bson::String(format!("s{}", rnd() % 30)));
                        }
                        5 => {
                            d.insert(f, Bson::Boolean(rnd() % 2 == 0));
                        }
                        6 => {
                            d.insert(f, Bson::DateTime(bson::DateTime::from_millis(
                                (rnd() % 1_000_000) as i64,
                            )));
                        }
                        7 => {
                            d.insert(f, Bson::Double(f64::NAN));
                        }
                        8 => {
                            d.insert(f, Bson::Double(f64::INFINITY));
                        }
                        _ => {
                            d.insert(f, Bson::Double(f64::NEG_INFINITY));
                        }
                    }
                }
                d.insert("_id", bson::oid::ObjectId::from_bytes([
                    0, 0, 0, 0, 0, 0, 0, (i >> 24) as u8, (i >> 16) as u8, (i >> 8) as u8,
                    i as u8, 0,
                ]));
                docs.push(d);
            }
            coll.insert_many(&docs).await.unwrap();

            let specs: Vec<Vec<(String, i8)>> = vec![
                vec![("a".into(), 1)],
                vec![("a".into(), -1)],
                vec![("b".into(), 1)],
                vec![("c".into(), -1)],
                vec![("a".into(), 1), ("b".into(), -1)],
                vec![("a".into(), -1), ("b".into(), 1), ("c".into(), 1)],
            ];
            let mut failures = 0;
            for raw in specs {
                let sort = normalize_sort(&raw);
                for limit in [1u32, 2, 3, 5, 13] {
                    // baseline: single full scan
                    let mut cur = coll
                        .find(Document::new())
                        .sort(sort_document(&sort))
                        .limit(200)
                        .await
                        .unwrap();
                    let mut baseline = Vec::new();
                    while let Some(d) = cur.try_next().await.unwrap() {
                        baseline.push(d.get_object_id("_id").unwrap().to_hex());
                    }
                    // paginated walk, exactly like the server
                    let mut got = Vec::new();
                    let mut cursor: Option<Cursor> = None;
                    loop {
                        let filter = build_filter(None, cursor.as_ref()).unwrap();
                        let mut cur = coll
                            .find(filter)
                            .sort(sort_document(&sort))
                            .limit(limit as i64 + 1)
                            .await
                            .unwrap();
                        let mut page = Vec::new();
                        while let Some(d) = cur.try_next().await.unwrap() {
                            page.push(d);
                            if page.len() as u32 > limit {
                                break;
                            }
                        }
                        let has_more = page.len() as u32 > limit;
                        if has_more {
                            page.truncate(limit as usize);
                        }
                        for d in &page {
                            got.push(d.get_object_id("_id").unwrap().to_hex());
                        }
                        if has_more {
                            let last = page.last().unwrap();
                            let mut vals = Vec::new();
                            for (f, _) in &sort {
                                vals.push(
                                    bson_to_cursor_json(
                                        &last.get(f).cloned().unwrap_or(Bson::Null),
                                    )
                                    .unwrap(),
                                );
                            }
                            cursor = Some(Cursor {
                                v: 1,
                                id: "t".into(),
                                db: "xdb_test".into(),
                                coll: coll_name.clone(),
                                sort: sort.clone(),
                                last: vals,
                            });
                        } else {
                            break;
                        }
                    }
                    if got != baseline {
                        failures += 1;
                        eprintln!(
                            "PAGINATION MISMATCH sort={sort:?} limit={limit}\n  baseline={:?}\n  got={:?}",
                            baseline, got
                        );
                    }
                }
            }
            coll.drop().await.unwrap();
            assert_eq!(failures, 0, "keyset pagination diverged from full scan");

            // phase 2: arrays in the sort field — pagination must either
            // complete with full equivalence or stop with the explicit
            // "array value" error (mirroring find_docs), never diverge
            // silently and never loop
            let coll2 = db.collection::<Document>(&format!("{coll_name}_arr"));
            coll2.drop().await.unwrap();
            let mut docs2 = Vec::new();
            for i in 0..30u32 {
                let mut d = Document::new();
                for f in ["a", "b"] {
                    match rnd() % 6 {
                        0 => {
                            d.insert(f, Bson::Null);
                        }
                        1 => {} // missing
                        2 => {
                            d.insert(f, Bson::Int32((rnd() % 100) as i32));
                        }
                        3 => {
                            d.insert(f, Bson::String(format!("s{}", rnd() % 20)));
                        }
                        4 => {
                            d.insert(f, Bson::Boolean(rnd() % 2 == 0));
                        }
                        _ => {
                            d.insert(f, Bson::Array(vec![
                                Bson::Int32((rnd() % 10) as i32),
                                Bson::String(format!("e{}", rnd() % 5)),
                            ]));
                        }
                    }
                }
                d.insert("_id", bson::oid::ObjectId::from_bytes([
                    0, 0, 0, 0, 0, 0, 0, (i >> 24) as u8, (i >> 16) as u8, (i >> 8) as u8,
                    i as u8, 0,
                ]));
                docs2.push(d);
            }
            coll2.insert_many(&docs2).await.unwrap();
            let mut array_failures = 0;
            for raw in [vec![("a".into(), 1)], vec![("a".into(), -1)], vec![("a".into(), 1), ("b".into(), -1)]] {
                let sort = normalize_sort(&raw);
                for limit in [1u32, 2, 5] {
                    let mut cur = match coll2
                        .find(Document::new())
                        .sort(sort_document(&sort))
                        .limit(200)
                        .await
                    {
                        Ok(c) => c,
                        Err(_) => continue, // parallel arrays: Mongo rejects the sort itself
                    };
                    let mut baseline = Vec::new();
                    while let Some(d) = cur.try_next().await.unwrap() {
                        baseline.push(d.get_object_id("_id").unwrap().to_hex());
                    }
                    let mut got = Vec::new();
                    let mut cursor: Option<Cursor> = None;
                    let mut explicit_stop = false;
                    for _page in 0..200 {
                        let filter = build_filter(None, cursor.as_ref()).unwrap();
                        let mut cur = coll2
                            .find(filter)
                            .sort(sort_document(&sort))
                            .limit(limit as i64 + 1)
                            .await
                            .unwrap();
                        let mut page = Vec::new();
                        while let Some(d) = cur.try_next().await.unwrap() {
                            page.push(d);
                            if page.len() as u32 > limit {
                                break;
                            }
                        }
                        let has_more = page.len() as u32 > limit;
                        if has_more {
                            page.truncate(limit as usize);
                        }
                        // mirror find_docs: array sort values → explicit stop
                        if has_more
                            && page
                                .iter()
                                .chain(cursor.as_ref().map(|_| &page[page.len() - 1]))
                                .any(|d| {
                                    sort.iter()
                                        .any(|(f, _)| matches!(d.get(f), Some(Bson::Array(_))))
                                })
                        {
                            explicit_stop = true;
                            break;
                        }
                        for d in &page {
                            got.push(d.get_object_id("_id").unwrap().to_hex());
                        }
                        if has_more {
                            let last = page.last().unwrap();
                            let mut vals = Vec::new();
                            for (f, _) in &sort {
                                vals.push(
                                    bson_to_cursor_json(
                                        &last.get(f).cloned().unwrap_or(Bson::Null),
                                    )
                                    .unwrap(),
                                );
                            }
                            cursor = Some(Cursor {
                                v: 1,
                                id: "t".into(),
                                db: "xdb_test".into(),
                                coll: format!("{coll_name}_arr"),
                                sort: sort.clone(),
                                last: vals,
                            });
                        } else {
                            break;
                        }
                    }
                    if !explicit_stop && got != baseline {
                        array_failures += 1;
                        eprintln!(
                            "ARRAY PAGINATION MISMATCH sort={sort:?} limit={limit}\n  baseline={:?}\n  got={:?}",
                            baseline, got
                        );
                    }
                }
            }
            coll2.drop().await.unwrap();
            assert_eq!(
                array_failures, 0,
                "array-field pagination diverged silently (must error or be exact)"
            );

            // phase 3: projection must not disturb pagination — the _id walk
            // stays identical to the unprojected baseline AND the visible
            // fields are exactly source ∩ expected for every variant
            let coll3 = db.collection::<Document>(&format!("{coll_name}_proj"));
            coll3.drop().await.unwrap();
            let mut docs3 = Vec::new();
            for i in 0..24u32 {
                let mut d = Document::new();
                d.insert("a", Bson::Int32((i % 7) as i32));
                d.insert("b", Bson::String(format!("s{}", i % 5)));
                if i % 3 != 0 {
                    d.insert("c", Bson::Double(i as f64 / 2.0)); // c sometimes missing
                }
                if i % 4 == 0 {
                    d.insert("x", Bson::Boolean(true)); // extra field
                }
                d.insert(
                    "_id",
                    bson::oid::ObjectId::from_bytes([
                        0, 0, 0, 0, 0, 0, 0, (i >> 24) as u8, (i >> 16) as u8, (i >> 8) as u8,
                        i as u8, 0,
                    ]),
                );
                docs3.push(d);
            }
            coll3.insert_many(&docs3).await.unwrap();
            let proj_variants: Vec<(&str, Option<Projection>)> = vec![
                ("none", None),
                (
                    "include_b",
                    parse_projection(&serde_json::json!({ "b": 1 })).unwrap(),
                ),
                (
                    "exclude_c_id",
                    parse_projection(&serde_json::json!({ "c": 0, "_id": 0 })).unwrap(),
                ),
            ];
            let mut proj_failures = 0;
            for raw in [
                vec![("a".into(), 1)],
                vec![("a".into(), -1), ("b".into(), 1)],
            ] {
                let sort = normalize_sort(&raw);
                for limit in [1u32, 5] {
                    // baseline: full scan keeping the full documents
                    let mut cur = coll3
                        .find(Document::new())
                        .sort(sort_document(&sort))
                        .limit(200)
                        .await
                        .unwrap();
                    let mut baseline: Vec<(String, Document)> = Vec::new();
                    while let Some(d) = cur.try_next().await.unwrap() {
                        baseline.push((d.get_object_id("_id").unwrap().to_hex(), d));
                    }
                    for (label, proj) in &proj_variants {
                        let mongo_proj = projection_document(proj.as_ref(), &sort);
                        let strip = projection_strip_fields(proj.as_ref(), &sort);
                        let all_keys: std::collections::BTreeSet<String> = baseline
                            .iter()
                            .flat_map(|(_, d)| d.keys().cloned())
                            .collect();
                        let expected: std::collections::BTreeSet<String> = match proj {
                            None => all_keys,
                            Some(p) => match p.style {
                                ProjectionStyle::Include => {
                                    let mut s = p.fields.clone();
                                    if p.include_id {
                                        s.insert("_id".into());
                                    }
                                    s
                                }
                                ProjectionStyle::Exclude => {
                                    let mut s = all_keys;
                                    for f in &p.fields {
                                        s.remove(f);
                                    }
                                    if p.exclude_id {
                                        s.remove("_id");
                                    }
                                    s
                                }
                            },
                        };
                        // paginated walk mirroring the server, projection applied
                        let mut got = Vec::new();
                        let mut cursor: Option<Cursor> = None;
                        loop {
                            let filter = build_filter(None, cursor.as_ref()).unwrap();
                            let mut f = coll3
                                .find(filter)
                                .sort(sort_document(&sort))
                                .limit(limit as i64 + 1);
                            if let Some(mp) = &mongo_proj {
                                f = f.projection(mp.clone());
                            }
                            let mut cur = f.await.unwrap();
                            let mut page = Vec::new();
                            while let Some(d) = cur.try_next().await.unwrap() {
                                page.push(d);
                                if page.len() as u32 > limit {
                                    break;
                                }
                            }
                            let has_more = page.len() as u32 > limit;
                            if has_more {
                                page.truncate(limit as usize);
                            }
                            for d in &page {
                                let id = d.get_object_id("_id").unwrap().to_hex();
                                got.push(id.clone());
                                // visible fields must be exactly source ∩ expected
                                let src = &baseline.iter().find(|(h, _)| *h == id).unwrap().1;
                                let visible = bson_to_json_projected(d, &strip);
                                let vis_keys: std::collections::BTreeSet<String> =
                                    visible.as_object().unwrap().keys().cloned().collect();
                                let src_keys: std::collections::BTreeSet<String> =
                                    src.keys().cloned().collect();
                                let want: std::collections::BTreeSet<String> =
                                    src_keys.intersection(&expected).cloned().collect();
                                if vis_keys != want {
                                    proj_failures += 1;
                                    eprintln!(
                                        "PROJECTION FIELD MISMATCH variant={label} sort={sort:?} limit={limit}: visible={vis_keys:?} want={want:?}"
                                    );
                                }
                            }
                            if has_more {
                                let last = page.last().unwrap();
                                let mut vals = Vec::new();
                                for (f, _) in &sort {
                                    vals.push(
                                        bson_to_cursor_json(
                                            &last.get(f).cloned().unwrap_or(Bson::Null),
                                        )
                                        .unwrap(),
                                    );
                                }
                                cursor = Some(Cursor {
                                    v: 1,
                                    id: "t".into(),
                                    db: "xdb_test".into(),
                                    coll: format!("{coll_name}_proj"),
                                    sort: sort.clone(),
                                    last: vals,
                                });
                            } else {
                                break;
                            }
                        }
                        let want_ids: Vec<String> =
                            baseline.iter().map(|(h, _)| h.clone()).collect();
                        if got != want_ids {
                            proj_failures += 1;
                            eprintln!(
                                "PROJECTION PAGINATION MISMATCH variant={label} sort={sort:?} limit={limit}"
                            );
                        }
                    }
                }
            }
            coll3.drop().await.unwrap();
            assert_eq!(
                proj_failures, 0,
                "projection changed pagination order or visible fields"
            );
        });
    }
}
