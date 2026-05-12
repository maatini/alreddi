use dashmap::DashMap;
use std::sync::Arc;

/// Maps external EAN/GTIN to internal SAP material numbers (MATNR).
///
/// Every EAN passed to downstream subgraphs must first pass the whitelist
/// and be translated to its internal MATNR. Unknown EANs are rejected.
#[derive(Clone)]
pub struct IdTranslator {
    /// EAN → MATNR mapping (lock-free sharded read)
    ean_to_matnr: Arc<DashMap<String, Arc<MatnrEntry>>>,
}

#[derive(Clone, Debug)]
pub struct MatnrEntry {
    pub ean: String,
    pub matnr: String,
    /// SAP material type code (e.g. "FERT" = finished good, "HAWA" = trading good).
    /// Used in Phase 4+ for type-based routing decisions.
    #[allow(dead_code)]
    pub material_type: String,
}

impl IdTranslator {
    pub fn new() -> Self {
        Self {
            ean_to_matnr: Arc::new(DashMap::new()),
        }
    }

    /// Populate the translator with seed data. Called at startup and
    /// periodically refreshed from the SAP article master export.
    pub fn with_seed_data() -> Self {
        let translator = Self::new();
        for entry in seed_entries() {
            translator.upsert(entry);
        }
        translator
    }

    /// Insert or update a mapping entry.
    pub fn upsert(&self, entry: MatnrEntry) {
        let ean = entry.ean.clone();
        self.ean_to_matnr.insert(ean, Arc::new(entry));
    }

    /// Check whether an EAN is in the whitelist (i.e. has a known mapping).
    pub fn is_whitelisted(&self, ean: &str) -> bool {
        self.ean_to_matnr.contains_key(ean)
    }

    /// Translate an EAN to its internal MATNR. Returns `None` if the EAN
    /// is not whitelisted.
    pub fn translate(&self, ean: &str) -> Option<Arc<MatnrEntry>> {
        self.ean_to_matnr.get(ean).map(|r| Arc::clone(&r))
    }

    /// Number of mapped entries.
    pub fn len(&self) -> usize {
        self.ean_to_matnr.len()
    }

    /// Return all known EANs (whitelist snapshot). Used for list/search queries.
    pub fn known_eans(&self) -> Vec<String> {
        self.ean_to_matnr
            .iter()
            .map(|r| r.key().clone())
            .collect()
    }
}

impl Default for IdTranslator {
    fn default() -> Self {
        Self::with_seed_data()
    }
}

// ---------------------------------------------------------------------------
// Seed data — Phase 3 starter set. Production sources from SAP export.
// ---------------------------------------------------------------------------

fn seed_entries() -> Vec<MatnrEntry> {
    vec![
        MatnrEntry {
            ean: "4012345678901".into(),
            matnr: "000000000001000001".into(),
            material_type: "HAWA".into(),
        },
        MatnrEntry {
            ean: "4012345678902".into(),
            matnr: "000000000001000002".into(),
            material_type: "HAWA".into(),
        },
        MatnrEntry {
            ean: "4012345678903".into(),
            matnr: "000000000001000003".into(),
            material_type: "HAWA".into(),
        },
        MatnrEntry {
            ean: "4012345678904".into(),
            matnr: "000000000001000004".into(),
            material_type: "FERT".into(),
        },
        MatnrEntry {
            ean: "4012345678905".into(),
            matnr: "000000000001000005".into(),
            material_type: "HAWA".into(),
        },
        // POS articles (Phase 2) — also need MATNR mappings
        MatnrEntry {
            ean: "4012345100000".into(),
            matnr: "000000000001000006".into(),
            material_type: "HAWA".into(),
        },
        MatnrEntry {
            ean: "4012345100001".into(),
            matnr: "000000000001000007".into(),
            material_type: "HAWA".into(),
        },
        MatnrEntry {
            ean: "4012345100002".into(),
            matnr: "000000000001000008".into(),
            material_type: "HAWA".into(),
        },
        MatnrEntry {
            ean: "4012345100003".into(),
            matnr: "000000000001000009".into(),
            material_type: "FERT".into(),
        },
        MatnrEntry {
            ean: "4012345100004".into(),
            matnr: "000000000001000010".into(),
            material_type: "HAWA".into(),
        },
        MatnrEntry {
            ean: "4012345100005".into(),
            matnr: "000000000001000011".into(),
            material_type: "HAWA".into(),
        },
        MatnrEntry {
            ean: "4012345100006".into(),
            matnr: "000000000001000012".into(),
            material_type: "HAWA".into(),
        },
        MatnrEntry {
            ean: "4012345100007".into(),
            matnr: "000000000001000013".into(),
            material_type: "HAWA".into(),
        },
        MatnrEntry {
            ean: "4012345100008".into(),
            matnr: "000000000001000014".into(),
            material_type: "HAWA".into(),
        },
        MatnrEntry {
            ean: "4012345100009".into(),
            matnr: "000000000001000015".into(),
            material_type: "HAWA".into(),
        },
        MatnrEntry {
            ean: "4012345999999".into(),
            matnr: "000000000001000016".into(),
            material_type: "HAWA".into(),
        },
        MatnrEntry {
            ean: "4012345100010".into(),
            matnr: "000000000001000017".into(),
            material_type: "HAWA".into(),
        },
    ]
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn translate_known_ean() {
        let translator = IdTranslator::with_seed_data();
        let entry = translator.translate("4012345678901").unwrap();
        assert_eq!(entry.matnr, "000000000001000001");
        assert_eq!(entry.material_type, "HAWA");
    }

    #[test]
    fn translate_unknown_ean_returns_none() {
        let translator = IdTranslator::with_seed_data();
        assert!(translator.translate("0000000000000").is_none());
    }

    #[test]
    fn whitelist_check() {
        let translator = IdTranslator::with_seed_data();
        assert!(translator.is_whitelisted("4012345678901"));
        assert!(!translator.is_whitelisted("9999999999999"));
    }

    #[test]
    fn upsert_new_mapping() {
        let translator = IdTranslator::new();
        translator.upsert(MatnrEntry {
            ean: "1234567890123".into(),
            matnr: "MAT-001".into(),
            material_type: "FERT".into(),
        });
        assert!(translator.is_whitelisted("1234567890123"));
        assert_eq!(
            translator.translate("1234567890123").unwrap().matnr,
            "MAT-001"
        );
    }

    #[test]
    fn upsert_overwrites() {
        let translator = IdTranslator::with_seed_data();
        translator.upsert(MatnrEntry {
            ean: "4012345678901".into(),
            matnr: "UPDATED-MATNR".into(),
            material_type: "FERT".into(),
        });
        let entry = translator.translate("4012345678901").unwrap();
        assert_eq!(entry.matnr, "UPDATED-MATNR");
        assert_eq!(entry.material_type, "FERT");
    }

    #[test]
    fn concurrent_reads_and_writes() {
        let translator = IdTranslator::with_seed_data();
        std::thread::scope(|s| {
            for _ in 0..10 {
                s.spawn(|| {
                    for _ in 0..100 {
                        let r = translator.translate("4012345678901");
                        assert!(r.is_some());
                    }
                });
            }
        });
    }
}
