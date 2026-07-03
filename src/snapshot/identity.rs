//! Stable app identity + provenance/trust tiers for the app-set snapshot.

/// Identity of an AL app, matching `app.json` / SymbolReference fields.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct AppId {
    pub guid: String,
    pub name: String,
    pub publisher: String,
    pub version: String,
}

impl AppId {
    /// Human-readable short form for logs/citations.
    pub fn short(&self) -> String {
        format!("{}/{}@{}", self.publisher, self.name, self.version)
    }
}

/// How trustworthy the source backing an app is.
///
/// `Hash`/`PartialOrd`/`Ord` (resolve-reason-split Task 2 addition): needed so
/// [`crate::program::resolve::edge::Route`]'s `receiver_tier: Option<TrustTier>`
/// diagnostic field can ride `Route`'s existing derived `Hash`/`Ord` stack.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum TrustTier {
    Workspace,
    EmbeddedSource,
    LocalSourceVerified,
    LocalSourceApproximate,
    SymbolOnly,
}

impl TrustTier {
    /// Higher = stronger evidence. Used for provider selection + honest claims.
    pub fn rank(self) -> u8 {
        match self {
            TrustTier::Workspace => 5,
            TrustTier::EmbeddedSource => 4,
            TrustTier::LocalSourceVerified => 3,
            TrustTier::LocalSourceApproximate => 2,
            TrustTier::SymbolOnly => 1,
        }
    }

    /// Stable snake_case identifier for diagnostic/export rendering (mirrors
    /// [`crate::program::resolve::edge::UnknownReason::as_str`]'s pattern) —
    /// the ONE canonical string mapping; `graphify_export::tier_str` and
    /// `aldump`'s `receiver_tier` rendering both delegate here rather than
    /// hand-rolling independent copies of the same match.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            TrustTier::Workspace => "workspace",
            TrustTier::EmbeddedSource => "embedded_source",
            TrustTier::LocalSourceVerified => "local_source_verified",
            TrustTier::LocalSourceApproximate => "local_source_approximate",
            TrustTier::SymbolOnly => "symbol_only",
        }
    }
}

/// Provenance attached to every snapshot node/unit.
#[derive(Clone, Debug)]
pub struct Provenance {
    pub app: AppId,
    pub tier: TrustTier,
    pub content_hash: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_id_equality_is_field_wise() {
        let a = AppId {
            guid: "4b915d7e-c02a-435f-85ab-649086c1e002".into(),
            name: "Continia Core".into(),
            publisher: "Continia Software".into(),
            version: "29.0.0.0".into(),
        };
        let b = a.clone();
        assert_eq!(a, b);
        assert_eq!(a.short(), "Continia Software/Continia Core@29.0.0.0");
    }

    #[test]
    fn trust_tier_orders_workspace_strongest() {
        assert!(TrustTier::Workspace.rank() > TrustTier::SymbolOnly.rank());
        assert!(TrustTier::EmbeddedSource.rank() > TrustTier::LocalSourceApproximate.rank());
    }
}
