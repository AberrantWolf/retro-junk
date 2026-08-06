//! Finding the company a name refers to, or creating it.
//!
//! Both enrichment sources — `GameTDB` and `ScreenScraper` — hand over a
//! publisher or developer as free text and need a row to point a release at.
//! They used to carry a copy each of the same lookup, which is how they drifted
//! into keying on a slug of the name: lowercase, ASCII letters and digits only.
//! That silently folded every company whose name has no ASCII in it — 「コナミ」
//! and 「カプコン」 both slug to the empty string — into one row.
//!
//! So the key is the name itself. An alias is tried first, because that is the
//! curated answer; an exact name match second, which is what reaches a company
//! seeded from YAML under a hand-chosen id; and only then is a row created,
//! with a minted id that carries no meaning and cannot collide.

use retro_junk_catalog::content_id;
use retro_junk_catalog::types::Company;
use retro_junk_db::operations::{self, OperationError};
use rusqlite::Connection;

/// What looking a company up did.
pub struct FoundCompany {
    pub id: String,
    /// True when no row described this name and one was made.
    pub created: bool,
}

/// The company this name refers to, creating it when nothing does yet.
pub fn find_or_create_company(
    conn: &Connection,
    name: &str,
) -> Result<FoundCompany, OperationError> {
    if let Some(company_id) = operations::find_company_by_alias(conn, name)? {
        return Ok(FoundCompany {
            id: company_id,
            created: false,
        });
    }
    let by_name: Option<String> = conn
        .query_row(
            "SELECT id FROM companies WHERE name = ?1 ORDER BY id LIMIT 1",
            [name],
            |row| row.get(0),
        )
        .map(Some)
        .or_else(|error| match error {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            other => Err(other),
        })?;
    if let Some(found) = by_name {
        return Ok(FoundCompany {
            id: found,
            created: false,
        });
    }

    // A company is a grouping with no content to fold, so its id is minted the
    // same way a work's is: unique by construction, meaningless by design.
    let company = Company {
        id: content_id::new_company_id(),
        name: name.to_owned(),
        country: String::new(),
        aliases: vec![name.to_owned()],
    };
    operations::upsert_company(conn, &company)?;
    log::debug!("Created new company: {name} ({})", company.id);
    Ok(FoundCompany {
        id: company.id,
        created: true,
    })
}
