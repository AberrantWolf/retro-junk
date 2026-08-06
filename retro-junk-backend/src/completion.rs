//! The one completion model.
//!
//! Every icon, badge, table cell, detail row, and CLI status line renders a
//! [`Completion`] computed by the backend. Nothing else may derive a status:
//! the old scheme had nine overlapping status types and two disagreeing SQL
//! definitions of "verified discs", and every contradiction the UI ever
//! showed lived in the seams between them.
//!
//! Two rules are enforced by construction:
//!
//! - A missing denominator is never rendered as `0/0`. [`Fraction`] is either
//!   a real measurement or [`Fraction::Unknown`] with the reason measurement
//!   is impossible — and the reason names the fix.
//! - "Gray" means the identity is unknown, never "a denominator happened to
//!   be zero". The overall fold ([`Completion::overall`]) can only report
//!   `Unidentified` from [`Identity`], not from a count.

#[cfg(test)]
#[path = "completion_tests.rs"]
mod completion_tests;

use serde::{Deserialize, Serialize};

/// How we know what a release is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Identity {
    /// Catalog-verified identity: content hashes matched a catalog entry.
    Bound { release_id: String },
    /// Hash-verified at the work level only: the carriers matched media from
    /// different mastering records of the same work, so no single catalog
    /// release describes the set.
    WorkBound { work_id: String },
    /// The archive verified these bytes against a catalog, and no entry in
    /// this machine's catalog matches them. Not "unbound" — the work was done
    /// and came back empty. The fix is a catalog this content is actually in,
    /// or accepting that nothing will ever list it.
    ContentUnmatched,
    /// Weak evidence only (serial, header, or filename): we have a display
    /// name but no hash-verified catalog identity.
    Named { name: String },
    /// Nothing identifies this content yet.
    Unknown,
}

impl Identity {
    /// The display name a frontend should show, when any evidence names one.
    #[must_use]
    pub fn display_name(&self) -> Option<&str> {
        match self {
            Self::Bound { release_id } => Some(release_id),
            Self::WorkBound { work_id } => Some(work_id),
            Self::Named { name } => Some(name),
            Self::ContentUnmatched | Self::Unknown => None,
        }
    }
}

/// Why a fraction cannot be measured. Each reason names its fix — frontends
/// render these instead of inventing their own apology strings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum UnknownReason {
    /// No catalog identity, so there is no expected count to measure
    /// against. Fix: identify against the catalog.
    NotCatalogBound,
    /// The archive matched these bytes against a catalog, and this machine's
    /// catalog has no entry with them. Fix: import a catalog that lists this
    /// content, if one does.
    ContentUnmatched,
    /// The catalog resolves the identity but lists no media for it, so
    /// there is no expected disc set to measure against. Fix: re-import
    /// the catalog for this platform.
    CatalogMissingMedia,
}

impl UnknownReason {
    /// One sentence: what is unknown and what makes it known.
    #[must_use]
    pub fn explain(self) -> &'static str {
        match self {
            Self::NotCatalogBound => {
                "No catalog identity yet — identify this release to set an expectation"
            }
            Self::ContentUnmatched => {
                "No catalog on this machine lists these bytes — import a catalog that covers them"
            }
            Self::CatalogMissingMedia => {
                "The catalog knows this release but lists no media for it — re-import the catalog for this platform"
            }
        }
    }
}

/// A measured have/want pair, or the reason measurement is impossible.
///
/// `Known { want: 0 }` is a legitimate value — "nothing is expected here"
/// (for example, a release whose playable policy asks for nothing). It is
/// NOT the same as `Unknown`, where we cannot even say what is expected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Fraction {
    Known { have: u64, want: u64 },
    Unknown(UnknownReason),
}

/// How complete one measured aspect is, for rendering. The variants map
/// one-to-one onto dot colors / cell text; no frontend re-derives them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FractionLevel {
    /// Everything expected is there.
    Complete,
    /// Some, not all.
    Partial,
    /// Expected, and none of it is there.
    Empty,
    /// Nothing is expected of this aspect, so there is nothing to show.
    ///
    /// Distinct from `Complete` on purpose: a cartridge release asks for no
    /// disc conversion, and painting its playable dot bright green would
    /// claim evidence that was never gathered. It still does not *block*
    /// completion — see [`Fraction::is_complete`].
    NotApplicable,
    /// Cannot be measured; carries the reason.
    Unknown(UnknownReason),
}

impl Fraction {
    #[must_use]
    pub fn known(have: u64, want: u64) -> Self {
        Self::Known { have, want }
    }

    /// Complete means "nothing expected is missing", so a fraction that
    /// expects nothing does not hold back the overall state. An unknown
    /// fraction is never complete — not knowing what to expect is not the
    /// same as having everything. (For display, "expects nothing" is its own
    /// level; see [`Fraction::level`].)
    #[must_use]
    pub fn is_complete(self) -> bool {
        match self {
            Self::Known { have, want } => have >= want,
            Self::Unknown(_) => false,
        }
    }

    #[must_use]
    pub fn level(self) -> FractionLevel {
        match self {
            Self::Known { want: 0, .. } => FractionLevel::NotApplicable,
            Self::Known { have, want } => {
                if have >= want {
                    FractionLevel::Complete
                } else if have == 0 {
                    FractionLevel::Empty
                } else {
                    FractionLevel::Partial
                }
            }
            Self::Unknown(reason) => FractionLevel::Unknown(reason),
        }
    }

    /// The one textual rendering of a fraction. `"3 of 4"`, `"nothing
    /// expected"`, or the unknown reason's explanation — never a bare
    /// `0 / 0`.
    #[must_use]
    pub fn describe(self) -> String {
        match self {
            Self::Known { want: 0, .. } => "nothing expected".to_owned(),
            Self::Known { have, want } => format!("{have} of {want}"),
            Self::Unknown(reason) => reason.explain().to_owned(),
        }
    }
}

/// Carriers whose bytes a catalog verified, but which this machine's catalog
/// cannot name.
///
/// The archive records that a catalog agreed with these bytes; the projection
/// then re-derived which medium that was, from the same digests, and found no
/// row. So the content is real and checked, and nothing here lists it.
fn unmatched_carriers(facts: &retro_junk_db::facts::ReleaseFacts) -> u64 {
    facts
        .carriers
        .iter()
        .filter(|carrier| carrier.catalog_verified && carrier.catalog_media_id.is_none())
        .count() as u64
}

/// An actionable problem attached to a release. Every variant names the fix
/// as data, so frontends offer the action instead of describing the state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Attention {
    /// A built playable's on-disk name is not what the current catalog
    /// would call it. Fix: the atomic rename repair.
    StaleName {
        representation_id: String,
        current: String,
        canonical: String,
    },
    /// Built playables are not where their evidence says. Fix: adopt (find
    /// them by content), never rebuild beside them.
    PlayableMissing { count: u64 },
    /// The archive checked these bytes against a catalog and nothing matched.
    /// Fix: import a catalog that lists this content.
    ContentUnmatched { carriers: u64 },
    /// A preservation master the manifest records is missing or has the
    /// wrong size on disk.
    MasterMissing { carrier_id: String },
}

/// The complete status of one release: identity plus one fraction per
/// aspect, plus every actionable problem. Computed in exactly one place
/// (the backend projection); rendered everywhere.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Completion {
    pub identity: Identity,
    /// Preservation-master dumps present on disk vs. recorded.
    pub presence: Fraction,
    /// Dumps whose stored bytes re-hash to their recorded evidence.
    pub integrity: Fraction,
    /// Discs hash-verified against the catalog vs. the catalog's expected
    /// disc count. Unknown unless the identity is `Bound`.
    pub catalog: Fraction,
    /// Playables built and present vs. what the policy asks for.
    pub playable: Fraction,
    /// Artwork types archived vs. the expected set.
    pub artwork: Fraction,
    pub attention: Vec<Attention>,
}

/// The single overall fold — the row icon. Order of precedence: identity
/// first (you cannot be "complete" if we don't know what you are), then
/// problems, then the fractions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Overall {
    /// Identity is `Unknown` or merely `Named`: nothing hash-verified says
    /// what this is.
    Unidentified,
    /// Identity claims exist but cannot be resolved, or something needs a
    /// human/repair action.
    NeedsAttention,
    /// Identified, no problems, but at least one aspect is not complete.
    Incomplete,
    /// Identified, no problems, every aspect complete.
    Complete,
}

/// How bad a state is, on the one scale every indicator renders through.
///
/// The general status column and the per-aspect evidence badges both map
/// through this, and an overall severity is the *worst* of the severities it
/// is made of ([`Completion::severity`]) rather than a second judgement made
/// beside them. That is what makes it impossible for the two columns to
/// disagree: the summary cannot be greener than its greenest part.
///
/// Ordered worst-last so `max` folds them. See `IDENTIFICATION.md` for the
/// colour and icon each one carries, and why there are five.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Severity {
    /// Complete and checked.
    Verified,
    /// A person's assertion is load-bearing: a manual disambiguation, or
    /// content no catalog will ever list (homebrew, a hack, a mod). Correct,
    /// but never machine-verifiable — so never green, and never a defect.
    Asserted,
    /// No measurement is possible yet, and the reason says why.
    Unmeasured,
    /// Usable, but something is missing or unverified.
    Incomplete,
    /// Unusable: nothing identifies it, or a hash contradicts what it claims
    /// to be.
    Broken,
}

impl Severity {
    /// The worse of two severities.
    #[must_use]
    pub fn worst(self, other: Self) -> Self {
        self.max(other)
    }

    /// One word, for a narrow cell or a CLI column.
    #[must_use]
    pub const fn short_label(self) -> &'static str {
        match self {
            Self::Verified => "Verified",
            Self::Asserted => "Asserted",
            Self::Unmeasured => "Unmeasured",
            Self::Incomplete => "Incomplete",
            Self::Broken => "Broken",
        }
    }
}

impl FractionLevel {
    /// Where this aspect sits on the one scale.
    ///
    /// `NotApplicable` is [`Severity::Verified`] rather than a state of its
    /// own: nothing is expected, so nothing is outstanding.
    #[must_use]
    pub const fn severity(self) -> Severity {
        match self {
            Self::Complete | Self::NotApplicable => Severity::Verified,
            Self::Partial | Self::Empty => Severity::Incomplete,
            Self::Unknown(_) => Severity::Unmeasured,
        }
    }
}

impl Identity {
    /// Where an identity sits on the one scale.
    #[must_use]
    pub const fn severity(&self) -> Severity {
        match self {
            Self::Bound { .. } | Self::WorkBound { .. } => Severity::Verified,
            // The disk names a catalog entry this machine lacks. Nothing is
            // wrong with the file; we simply cannot judge it until the catalog
            // is imported, which is what "unmeasured" means.
            Self::ContentUnmatched => Severity::Unmeasured,
            // A name with no hash-verified identity behind it is not a
            // weaker kind of known — it is unusable, because acting on it
            // would act on a guess.
            Self::Named { .. } | Self::Unknown => Severity::Broken,
        }
    }
}

impl Overall {
    /// Where an overall state sits on the one scale.
    #[must_use]
    pub const fn severity(self) -> Severity {
        match self {
            Self::Complete => Severity::Verified,
            Self::Incomplete => Severity::Incomplete,
            Self::NeedsAttention | Self::Unidentified => Severity::Broken,
        }
    }

    /// The one label for an overall state. Frontends render this rather than
    /// writing their own wording, so the table, the detail panel, and the
    /// CLI cannot describe the same release differently.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Complete => "Complete and catalog-verified",
            Self::Incomplete => "Incomplete",
            Self::NeedsAttention => "Needs attention",
            Self::Unidentified => "Not identified",
        }
    }

    /// A one-word form for narrow table cells.
    #[must_use]
    pub fn short_label(self) -> &'static str {
        match self {
            Self::Complete => "Complete",
            Self::Incomplete => "Incomplete",
            Self::NeedsAttention => "Attention",
            Self::Unidentified => "Unidentified",
        }
    }
}

impl Completion {
    /// The one severity for this release, folded from the same evidence the
    /// badges render.
    ///
    /// The summary is the worst of its parts, so a release cannot show a green
    /// dot beside an amber badge. Anything a person must decide about lands on
    /// [`Severity::Broken`] through `attention`, because an unresolved
    /// decision is the tool declining to act.
    #[must_use]
    pub fn severity(&self) -> Severity {
        let mut severity = self.identity.severity();
        for fraction in self.evidence() {
            severity = severity.worst(fraction.level().severity());
        }
        if !self.attention.is_empty() {
            severity = severity.worst(Severity::Broken);
        }
        severity
    }

    /// Every aspect that carries evidence, in the order frontends show them.
    ///
    /// One list, so a new aspect cannot be added to the badges and forgotten
    /// in the fold — which is how a summary drifts away from its parts.
    #[must_use]
    pub const fn evidence(&self) -> [Fraction; 5] {
        [
            self.presence,
            self.integrity,
            self.catalog,
            self.playable,
            self.artwork,
        ]
    }

    #[must_use]
    pub fn overall(&self) -> Overall {
        match &self.identity {
            Identity::Unknown | Identity::Named { .. } => return Overall::Unidentified,
            Identity::ContentUnmatched => return Overall::NeedsAttention,
            Identity::Bound { .. } | Identity::WorkBound { .. } => {}
        }
        if !self.attention.is_empty() {
            return Overall::NeedsAttention;
        }
        let all = self.evidence();
        if all.iter().all(|fraction| fraction.is_complete()) {
            Overall::Complete
        } else {
            Overall::Incomplete
        }
    }

    /// The one place a release's facts become a status.
    ///
    /// `expected_assets` is the artwork policy: which asset types a complete
    /// release archives. Everything else comes from the projection's facts.
    #[must_use]
    #[allow(clippy::too_many_lines)]
    pub fn for_release(
        facts: &retro_junk_db::facts::ReleaseFacts,
        expected_assets: &retro_junk_frontend::AssetSelection,
    ) -> Self {
        let identity = if let Some(release_id) = &facts.catalog_release_id {
            Identity::Bound {
                release_id: release_id.clone(),
            }
        } else if let Some(work_id) = &facts.catalog_work_id {
            Identity::WorkBound {
                work_id: work_id.clone(),
            }
        } else if unmatched_carriers(facts) > 0 {
            // Carriers whose dumps were checked against a catalog and matched,
            // yet resolve to no row here: this machine's catalog does not list
            // what these bytes are. That is a different problem from never
            // having looked, and it has a different fix.
            Identity::ContentUnmatched
        } else if facts.title.is_empty() {
            Identity::Unknown
        } else {
            Identity::Named {
                name: facts.title.clone(),
            }
        };

        let masters_recorded: u64 = facts.carriers.iter().map(|c| c.masters_recorded).sum();
        let masters_present: u64 = facts.carriers.iter().map(|c| c.masters_present).sum();
        let presence = Fraction::known(masters_present, masters_recorded);

        let dumped_carriers = facts
            .carriers
            .iter()
            .filter(|c| c.masters_recorded > 0)
            .count() as u64;
        let integrity_verified = facts
            .carriers
            .iter()
            .filter(|c| c.masters_recorded > 0 && c.integrity_verified)
            .count() as u64;
        let integrity = Fraction::known(integrity_verified, dumped_carriers);

        let catalog = match (&identity, facts.expected_discs) {
            (Identity::Bound { .. } | Identity::WorkBound { .. }, Some(expected)) => {
                Fraction::known(
                    retro_junk_db::facts::verified_disc_count(facts),
                    expected.count,
                )
            }
            (Identity::Bound { .. } | Identity::WorkBound { .. }, None) => {
                Fraction::Unknown(UnknownReason::CatalogMissingMedia)
            }
            (Identity::ContentUnmatched, _) => Fraction::Unknown(UnknownReason::ContentUnmatched),
            (Identity::Named { .. } | Identity::Unknown, _) => {
                Fraction::Unknown(UnknownReason::NotCatalogBound)
            }
        };

        let playable = Fraction::known(facts.satisfied_playables, facts.desired_playables);

        let held: std::collections::HashSet<retro_junk_frontend::AssetType> = facts
            .archived_asset_types
            .iter()
            .filter_map(|name| retro_junk_frontend::AssetType::from_archive_name(name))
            .collect();
        let artwork_have = expected_assets
            .types
            .iter()
            .filter(|asset_type| held.contains(asset_type))
            .count() as u64;
        let artwork = Fraction::known(artwork_have, expected_assets.types.len() as u64);

        let mut attention = Vec::new();
        if identity == Identity::ContentUnmatched {
            attention.push(Attention::ContentUnmatched {
                carriers: unmatched_carriers(facts),
            });
        }
        for carrier in &facts.carriers {
            if carrier.masters_present < carrier.masters_recorded {
                attention.push(Attention::MasterMissing {
                    carrier_id: carrier.carrier_id.clone(),
                });
            }
        }
        if facts.missing_playables > 0 {
            attention.push(Attention::PlayableMissing {
                count: facts.missing_playables,
            });
        }
        attention.extend(stale_names(facts));

        Self {
            identity,
            presence,
            integrity,
            catalog,
            playable,
            artwork,
            attention,
        }
    }
}

/// Built playables whose name is no longer what the catalog calls them.
///
/// This is the check that makes a playable built under an older naming rule
/// findable. It only ever speaks when the catalog does: an unidentified
/// release has no authority to call its own file misnamed, which
/// `naming::name_conforms` enforces.
fn stale_names(facts: &retro_junk_db::facts::ReleaseFacts) -> Vec<Attention> {
    let disc_count = facts
        .expected_discs
        .map_or(0, |expected| u32::try_from(expected.count).unwrap_or(0));
    facts
        .playable_names
        .iter()
        .filter_map(|playable| {
            let current = std::path::Path::new(&playable.relative_path)
                .file_name()
                .and_then(|name| name.to_str())?;
            let inputs = retro_junk_lib::naming::NameInputs {
                dat_name: &playable.dat_name,
                rom_name: &playable.rom_name,
                medium_has_tracks: playable.medium_has_tracks,
                title: &facts.title,
                region: &facts.region,
                revision: &facts.revision,
                variant: "",
                disc_number: playable.disc_number,
                disc_count,
            };
            if retro_junk_lib::naming::name_conforms(current, &inputs) {
                return None;
            }
            let extension = std::path::Path::new(current)
                .extension()
                .and_then(|value| value.to_str())
                .unwrap_or_default();
            Some(Attention::StaleName {
                representation_id: playable.representation_id.clone(),
                current: current.to_owned(),
                canonical: retro_junk_lib::naming::canonical_filename(&inputs, extension),
            })
        })
        .collect()
}
