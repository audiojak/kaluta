//! Analysis settings (spec §14.10), per account in `analysis_meta`: the
//! daily review, where facts come from, the cost cap and how long AI
//! drafts are kept. And which accounts have something new (the account
//! menu's dots).

use mail_store::analysis as store;

use crate::analysis_glean::{FactsFrom, keys as glean};
use crate::analysis_run::{DEFAULT_PAIRS_PER_DAY, keys};
use crate::{Core, CoreError, ErrorKind, runtime};

/// Days AI drafts' full texts are kept after review, unless changed.
pub const DEFAULT_KEEP_DAYS: u32 = 30;
/// The choices for keeping them.
pub const KEEP_DAYS: [u32; 3] = [7, 30, 90];
/// `analysis_meta` key: days to keep full texts.
pub(crate) const KEEP_DAYS_KEY: &str = "keep_days";

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct AnalysisSettings {
    pub daily_review: bool,
    pub facts_from: FactsFrom,
    /// Pairs compared a day at most (the cost cap).
    pub pairs_per_day: u32,
    /// Days the full texts are kept after review: 7, 30 or 90.
    pub keep_days: u32,
}

#[uniffi::export]
impl Core {
    pub async fn analysis_settings(&self) -> Result<AnalysisSettings, CoreError> {
        let db = self.db()?;
        runtime::run(async move {
            Ok(db
                .read(|c| {
                    let get = |k: &str| store::meta(c, k);
                    Ok(AnalysisSettings {
                        daily_review: get(keys::DAILY_REVIEW)?.is_none_or(|v| v != "off"),
                        facts_from: FactsFrom::parse(get(glean::FACTS_FROM)?.as_deref()),
                        pairs_per_day: get(keys::PAIRS_PER_DAY)?
                            .and_then(|v| v.parse().ok())
                            .unwrap_or(DEFAULT_PAIRS_PER_DAY),
                        keep_days: get(KEEP_DAYS_KEY)?
                            .and_then(|v| v.parse().ok())
                            .filter(|d| KEEP_DAYS.contains(d))
                            .unwrap_or(DEFAULT_KEEP_DAYS),
                    })
                })
                .await?)
        })
        .await
    }

    pub async fn set_analysis_settings(&self, settings: AnalysisSettings) -> Result<(), CoreError> {
        if !KEEP_DAYS.contains(&settings.keep_days) {
            return Err(CoreError::new(ErrorKind::InvalidInput, "keep AI drafts for 7, 30 or 90 days"));
        }
        if !(1..=500).contains(&settings.pairs_per_day) {
            return Err(CoreError::new(ErrorKind::InvalidInput, "compare between 1 and 500 drafts a day"));
        }
        let db = self.db()?;
        runtime::run(async move {
            Ok(db
                .write(move |tx| {
                    store::set_meta(tx, keys::DAILY_REVIEW, if settings.daily_review { "on" } else { "off" })?;
                    store::set_meta(tx, glean::FACTS_FROM, settings.facts_from.as_str())?;
                    store::set_meta(tx, keys::PAIRS_PER_DAY, &settings.pairs_per_day.to_string())?;
                    store::set_meta(tx, KEEP_DAYS_KEY, &settings.keep_days.to_string())
                })
                .await?)
        })
        .await
    }

    /// Accounts with proposals shown since the user last opened their
    /// Analysis: the account menu's dots (spec §14.10).
    pub async fn accounts_with_unseen_analysis(&self) -> Vec<String> {
        let mut out = Vec::new();
        let open: Vec<String> =
            self.open_accounts.read().unwrap_or_else(|e| e.into_inner()).stores.keys().cloned().collect();
        for account in open {
            let unseen = crate::registry::scoped(Some(account.clone()), async {
                self.analysis_queue().await.map(|q| q.unseen).unwrap_or(false)
            })
            .await;
            if unseen {
                out.push(account);
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use futures::executor::block_on;

    use super::*;
    use crate::analysis_run::tests::{learned, rt};

    #[test]
    fn settings_round_trip_and_take_effect() {
        let s = crate::guide::tests::demo("analysis-settings");
        let core = &s.1;
        core.debug_use_fake_agents();
        let first = block_on(core.analysis_settings()).unwrap();
        assert_eq!(
            first,
            AnalysisSettings {
                daily_review: true,
                facts_from: FactsFrom::MailWrittenWithAi,
                pairs_per_day: 50,
                keep_days: 30
            }
        );
        let off =
            AnalysisSettings { daily_review: false, facts_from: FactsFrom::Off, pairs_per_day: 20, keep_days: 90 };
        block_on(core.set_analysis_settings(off.clone())).unwrap();
        assert_eq!(block_on(core.analysis_settings()).unwrap(), off);
        assert!(block_on(core.set_analysis_settings(AnalysisSettings { keep_days: 10, ..off.clone() })).is_err());
        assert!(block_on(core.set_analysis_settings(AnalysisSettings { pairs_per_day: 0, ..off })).is_err());

        // The daily review off: the schedule starts nothing.
        learned(core);
        rt(core.analysis_tick(mail_sync::now_millis()));
        assert!(block_on(core.analysis_progress()).unwrap().run.is_none());
    }

    #[test]
    fn accounts_with_something_new_are_listed() {
        let s = crate::guide::tests::demo("analysis-dots");
        let core = &s.1;
        core.debug_use_fake_agents();
        learned(core);
        assert!(block_on(core.accounts_with_unseen_analysis()).is_empty());
        block_on(core.debug_seed_analysis()).unwrap();
        assert_eq!(block_on(core.accounts_with_unseen_analysis()), vec!["demo".to_owned()]);
        block_on(core.analysis_seen()).unwrap();
        assert!(block_on(core.accounts_with_unseen_analysis()).is_empty());
    }
}
