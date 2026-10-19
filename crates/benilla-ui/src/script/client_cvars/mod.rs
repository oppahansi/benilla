//! The client's CVar table: every CVar the reference registers that Lua can see, with its
//! registered default, as the rows a host seeds a VM's mirror with ([`client_table`]). A bare
//! [`UiScript::new`](super::UiScript::new) registers only this crate's own rows, and the stock
//! `UIOptionsFrame.xml`, `SoundOptionsFrame.xml` and `ContainerFrame.lua` read others while they
//! load, so a host registers this table before it loads the interface.
//!
//! - [`REGISTERED`] holds the vars benilla backs, a host knob or a Lua consumer, and [`INERT`] the
//!   reference's other script-visible ones, which `GetCVar` and `SetCVar` would otherwise raise
//!   on. A row's default is the reference's, and [`Registered::reference`] says where it stands
//!   against it.
//! - The live values, the saved file and the change callbacks are the host's: benilla-app's
//!   `cvars` registry reads this table, as does any other host.

#[cfg(test)]
mod deviations_doc;
mod inert;
mod table;

pub use inert::INERT;
pub use table::{all_rows, client_table, Reference, Registered, REGISTERED};

use super::CAMERA_VIEW_COUNT;

/// The logon server address's CVar, the reference's spelling.
pub const CVAR_REALMLIST: &str = "realmList";

/// [`CVAR_REALMLIST`]'s default. Deviation: the local machine's server, not the reference's
/// `us.logon.worldofwarcraft.com:3724`, because that host no longer resolves.
pub const DEFAULT_REALMLIST: &str = "localhost";

/// The live camera view's CVar (`0x84fdc8`), holding the internal index as `"%d"` (`0x512ef1`).
pub const CVAR_ACTIVE_VIEW: &str = "cameraView";

/// [`CVAR_ACTIVE_VIEW`]'s default (`[0x84f484]`): internal index 1, `THIRD_PERSON_A`.
pub const ACTIVE_VIEW_DEFAULT: &str = "1";

/// The fifteen saved-view CVars, `[view][field]` (distance, pitch, yaw), as `0x50b9b0` composes
/// them; [`REGISTERED`] holds the same names (tested in benilla-app).
pub const VIEW_CVARS: [[&str; 3]; CAMERA_VIEW_COUNT as usize] = [
    ["cameraDistance", "cameraPitch", "cameraYaw"],
    ["cameraDistanceA", "cameraPitchA", "cameraYawA"],
    ["cameraDistanceB", "cameraPitchB", "cameraYawB"],
    ["cameraDistanceC", "cameraPitchC", "cameraYawC"],
    ["cameraDistanceD", "cameraPitchD", "cameraYawD"],
];

/// The reference's saved-view default strings (`0x84f488`), kept as text: a host's saved file is
/// a diff against the registered default string, so a reset strips the key only by writing this
/// text.
pub const VIEW_DEFAULTS: [[&str; 3]; CAMERA_VIEW_COUNT as usize] = [
    ["0.0", "0.0", "0.0"],
    ["5.55", "10.0", "0.0"],
    ["5.55", "20.0", "0.0"],
    ["13.88", "30.0", "0.0"],
    ["13.88", "10.0", "0.0"],
];

/// `CombatLogPeriodicSpells`, the combat log's periodic-spell switch.
pub const LOG_PERIODIC_CVAR: &str = "CombatLogPeriodicSpells";

/// `CombatDeathLogRange`, registered at `0x626d5f` with default `"60"` (`0x862e14`).
pub const DEATH_LOG_RANGE_CVAR: &str = "CombatDeathLogRange";

/// The CVar the character select screen remembers the character entered as by: registered at
/// `0x402d93` (name `0x82e8f8`, default `"0"`, pointer at `[0x882674]`), written by `EnterWorld`
/// (`0x46b5fa`), named by no GlueXML.
pub const CVAR_LAST_CHARACTER: &str = "lastCharacterIndex";

#[cfg(test)]
mod tests {
    use super::*;

    /// A host gets every row once, at its locale's default: koKR's and zhCN's rows differ from
    /// enUS's only where a [`Reference::SameByLocale`] row says so.
    #[test]
    fn the_client_table_is_every_row_at_the_locales_default() {
        let en: Vec<_> = client_table(0).collect();
        assert_eq!(en.len(), REGISTERED.len() + INERT.len());
        let mut names: Vec<String> = en.iter().map(|(n, _)| n.to_ascii_lowercase()).collect();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), en.len(), "a name registered twice");
        for locale in [1, 4] {
            for ((name, here), row) in client_table(locale).zip(all_rows()) {
                assert_eq!(name, row.name);
                if here != row.default {
                    assert!(
                        matches!(row.reference, Reference::SameByLocale { .. }),
                        "{name}: only a by-locale row moves with the locale"
                    );
                }
            }
        }
        assert!(en.contains(&("gxRefresh", "75")));
        assert!(en.contains(&(CVAR_REALMLIST, DEFAULT_REALMLIST)));
    }
}
