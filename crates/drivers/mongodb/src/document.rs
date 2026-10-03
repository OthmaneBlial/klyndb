use base64::{Engine, engine::general_purpose::STANDARD};
use bson::{Bson, Document, doc};
use klyndb_driver_api::{DocumentChange, DocumentRecord, Error, Result};

pub const MAX_DOCUMENT: usize = 1024 * 1024;
pub const PAGE: usize = 100;
pub fn name(value: &str, database: bool) -> Result<()> {
    if value.is_empty()
        || value.len() > 255
        || value.contains('\0')
        || (database && value.contains(['/', '\\', '.', ' ', '"', '$']))
    {
        return Err(Error::new(
            "Enter a valid database or collection name (1–255 bytes)",
        ));
    }
    Ok(())
}
pub fn json(text: &str) -> Result<Bson> {
    if text.len() > MAX_DOCUMENT {
        return Err(Error::new("A document or query is limited to 1 MiB"));
    }
    let value: serde_json::Value = serde_json::from_str(text)
        .map_err(|_| Error::new("Enter valid JSON or MongoDB Extended JSON"))?;
    // serde_json arbitrary_precision must never silently demote a large integer to f64.
    fn numbers(value: &serde_json::Value) -> Result<()> {
        match value {
            serde_json::Value::Number(n)
                if !n.to_string().contains(['.', 'e', 'E']) && n.as_i64().is_none() =>
            {
                Err(Error::new(
                    "Integers must fit signed 64-bit BSON; use $numberDecimal for larger values",
                ))
            }
            serde_json::Value::Array(a) => a.iter().try_for_each(numbers),
            serde_json::Value::Object(o) => o.values().try_for_each(numbers),
            _ => Ok(()),
        }
    }
    numbers(&value)?;
    Bson::try_from(value).map_err(|_| Error::new("Invalid BSON Extended JSON value"))
}
pub fn object(text: &str) -> Result<Document> {
    match json(text)? {
        Bson::Document(d) => {
            if d.to_vec()
                .map_err(|_| Error::new("Invalid BSON document"))?
                .len()
                > MAX_DOCUMENT
            {
                return Err(Error::new("Encoded BSON document exceeds 1 MiB"));
            }
            Ok(d)
        }
        _ => Err(Error::new("Expected a JSON object")),
    }
}
pub fn read_only(value: &Bson) -> Result<()> {
    match value {
        Bson::Document(d) => {
            for (key, value) in d {
                if matches!(
                    key.as_str(),
                    "$out" | "$merge" | "$where" | "$function" | "$accumulator"
                ) {
                    return Err(Error::new(
                        "Document queries cannot write collections or run server-side JavaScript",
                    ));
                }
                read_only(value)?;
            }
        }
        Bson::Array(a) => {
            for value in a {
                read_only(value)?;
            }
        }
        _ => {}
    }
    Ok(())
}
pub fn pipeline(text: &str) -> Result<Vec<Document>> {
    let value = json(text)?;
    read_only(&value)?;
    let Bson::Array(stages) = value else {
        return Err(Error::new("An aggregation pipeline must be a JSON array"));
    };
    if stages.len() > 100 {
        return Err(Error::new("Use at most 100 aggregation stages"));
    }
    stages
        .into_iter()
        .map(|stage| match stage {
            Bson::Document(d)
                if d.len() == 1
                    && d.keys().next().is_some_and(|key| {
                        matches!(
                            key.as_str(),
                            "$match"
                                | "$project"
                                | "$group"
                                | "$sort"
                                | "$limit"
                                | "$skip"
                                | "$unwind"
                                | "$lookup"
                                | "$facet"
                                | "$replaceRoot"
                                | "$replaceWith"
                                | "$addFields"
                                | "$set"
                                | "$unset"
                                | "$count"
                                | "$sortByCount"
                                | "$bucket"
                                | "$bucketAuto"
                                | "$sample"
                                | "$setWindowFields"
                                | "$unionWith"
                                | "$densify"
                                | "$fill"
                                | "$geoNear"
                        )
                    }) =>
            {
                Ok(d)
            }
            _ => Err(Error::new(
                "Use one supported read-only aggregation operator per stage",
            )),
        })
        .collect()
}
pub fn record(document: Document, editable: bool) -> Result<DocumentRecord> {
    let bytes = document
        .to_vec()
        .map_err(|_| Error::new("Could not encode BSON document"))?;
    if bytes.len() > MAX_DOCUMENT {
        return Err(Error::new(
            "A returned document exceeds the 1 MiB workspace limit; narrow your projection",
        ));
    }
    let snapshot = (editable && document.contains_key("_id")).then(|| STANDARD.encode(bytes));
    let json = serde_json::to_string_pretty(&Bson::Document(document).into_canonical_extjson())
        .map_err(|_| Error::new("Could not encode Extended JSON"))?;
    Ok(DocumentRecord { json, snapshot })
}
pub fn original(snapshot: &str) -> Result<Document> {
    if snapshot.len() > MAX_DOCUMENT * 2 {
        return Err(Error::new("Original document exceeds 1 MiB"));
    }
    let bytes = STANDARD
        .decode(snapshot)
        .map_err(|_| Error::new("Invalid original document snapshot"))?;
    if bytes.len() > MAX_DOCUMENT {
        return Err(Error::new("Original document exceeds 1 MiB"));
    }
    let mut remaining = bytes.as_slice();
    let doc = Document::from_reader(&mut remaining)
        .map_err(|_| Error::new("Invalid original BSON snapshot"))?;
    if !remaining.is_empty() {
        return Err(Error::new("Unexpected trailing BSON snapshot bytes"));
    }
    if !doc.contains_key("_id") {
        return Err(Error::new("Document editing requires an original _id"));
    }
    Ok(doc)
}
pub fn predicate(original: Document) -> Result<Document> {
    let id = original
        .get("_id")
        .cloned()
        .ok_or_else(|| Error::new("Document editing requires _id"))?;
    // MongoDB's native document equality compares field order and values, with native numeric equality.
    Ok(doc! { "_id": id, "$expr": { "$eq": ["$$ROOT", { "$literal": original }] } })
}
pub fn validate_change(change: &DocumentChange) -> Result<()> {
    match change {
        DocumentChange::Insert { json } => {
            object(json)?;
        }
        DocumentChange::Replace { snapshot, json } => {
            let old = original(snapshot)?;
            let new = object(json)?;
            if new.get("_id") != old.get("_id") {
                return Err(Error::new(
                    "Keep the original _id when replacing a document",
                ));
            }
        }
        DocumentChange::Delete { snapshot } => {
            original(snapshot)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn extended_json_snapshot_and_query_guards() {
        let ordered = object(r#"{"z_order":1,"a_order":-1}"#).unwrap();
        assert_eq!(
            ordered.keys().map(String::as_str).collect::<Vec<_>>(),
            ["z_order", "a_order"]
        );
        let stages = pipeline(r#"[{"$sort":{"z_order":1,"a_order":-1}}]"#).unwrap();
        assert_eq!(stages[0].get_document("$sort").unwrap(), &ordered);
        let d = object(r#"{"_id":{"$oid":"507f1f77bcf86cd799439011"},"exact":{"$numberLong":"9223372036854775807"},"decimal":{"$numberDecimal":"1.0000000000000000001"},"bytes":{"$binary":{"base64":"YQD/","subType":"00"}},"date":{"$date":{"$numberLong":"1700000000000"}}}"#).unwrap();
        let row = record(d.clone(), true).unwrap();
        assert_eq!(original(row.snapshot.as_ref().unwrap()).unwrap(), d);
        assert_eq!(object(&row.json).unwrap(), d);
        assert!(json("9223372036854775808").is_err());
        assert!(pipeline(r#"[{"$facet":{"bad":[{"$merge":"target"}]}}]"#).is_err());
        assert!(pipeline(r#"[{"$match":{"$where":"secret"}}]"#).is_err());
        assert!(pipeline(r#"[{"$match":{}},{"$count":"total"}]"#).is_ok());
        assert!(pipeline(r#"[{"$currentOp":{}}]"#).is_err());
        let edit = DocumentChange::Replace {
            snapshot: row.snapshot.unwrap(),
            json: r#"{"_id":3}"#.into(),
        };
        assert!(validate_change(&edit).is_err());
    }
}
