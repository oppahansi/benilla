//! The interface load walk, engine-free: the core, the chain's own stock FrameXML as its
//! `FrameXML.toc` lists it ([`load_core`]) and then its `Bindings.xml` ([`load_core_bindings`]),
//! then benilla's layer ([`load_layer`]), every entry of which loads after the whole core, then
//! `UIParent_ManageFramePositions` ([`bootstrap_positions`]). [`load_interface`] runs the four in
//! that order over two file sources; [`load_files_into`] is the walk over one manifest's rows,
//! which a host's addons go through too.
//!
//! Nothing here reads a file or writes a log: the files come from the host's [`Read`]s, and every
//! line a host logs comes through its [`Log`].

use std::time::{Duration, Instant};

use crate::script::UiScript;
use crate::status::{self, Status};
use crate::toc::Toc;

/// The stock interface's table of contents, which `UI_Init` (`0x48fbf0`) hands to the `.toc`
/// runner (`0x6edb90`) at `0x48ffed`, once per UI build, `ReloadUI` included.
pub const FRAMEXML_TOC: &str = r"Interface\FrameXML\FrameXML.toc";

/// The stock key-binding commands, which `UI_Init` loads after the [`FRAMEXML_TOC`] walk
/// (`0x490018`).
pub const BINDINGS: &str = r"Interface\FrameXML\Bindings.xml";

/// benilla's own interface, one layer on the stock one, by its name in the layer's flat tree.
pub const LAYER_MANIFEST: &str = "layer.toc";

/// The name the stock files load under. FrameXML is not an addon: it gets no `ADDON_LOADED`, and
/// its chunks are named by chain path ([`Chunks::Path`]), as the 1.12 client does.
pub const CORE_NAME: &str = "FrameXML";

/// The name benilla's layer loads under.
pub const LAYER_NAME: &str = "benilla";

/// A file source: the bytes at a path, or `None`. Bytes, not text: a `.lua` reaches Lua as
/// stored, and a cp1252 file is not valid UTF-8.
pub type Read<'a> = &'a dyn Fn(&str) -> Option<Vec<u8>>;

/// A line for the host's log, at its severity, already worded.
pub type Log<'a> = &'a mut dyn FnMut(Severity, &str);

/// A log line's severity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Severity {
    Info,
    Warn,
    Error,
}

/// How a `.lua` row's chunk is named.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Chunks {
    /// `"@%s"` (`0x8716e0`) over the resolved install path, built by `0x704bc0` for every `.lua`,
    /// as for `<Script file=>`: the chain's files and a player's addon. Ace2 libraries find their
    /// addon by splitting a `debugstack` frame on `\AddOns\` (`AceDB-2.0.lua:742`).
    Path,
    /// [`crate::script::addon_chunk_name`]: benilla's flat shipped tree.
    Addon,
}

/// One interface's files, as the walk reads them.
pub struct Files<'a> {
    /// The name failures are tagged with (`"<name>/<file>: <error>"`) and a flat tree's chunks
    /// are named under.
    pub name: &'a str,
    /// The base the manifest's rows resolve against (`Interface/AddOns/<Folder>` for a player's
    /// addon, empty for full chain paths and the flat tree), `/`-separated as
    /// [`crate::loader::join_ref`] writes it.
    pub prefix: &'a str,
    /// The source, by a path already resolved into its path space; also the `<Include>` and
    /// `<Script file=>` provider, so it is the sandbox.
    pub read: Read<'a>,
    pub chunks: Chunks,
    /// Whether a missing file is the manifest's owner's error (ours, or a `FrameXML.toc` row the
    /// chain lacks: [`Severity::Error`]) or a player's addon's packaging, which the reference skips
    /// silently ([`Severity::Warn`]).
    pub strict: bool,
}

/// One manifest row's load.
#[derive(Clone, Debug)]
pub struct RowLoad {
    /// The row as its manifest names it.
    pub file: String,
    /// Frames its document materialized (0 for a `.lua`, or a row that failed to open or parse).
    pub frames: usize,
    /// Its failures, tagged `"<name>/<file>: <error>"`.
    pub failures: Vec<String>,
    /// What it cost.
    pub elapsed: Duration,
}

/// What a walk did.
#[derive(Clone, Debug, Default)]
pub struct Walk {
    /// Every row, in load order: the core's, then the layer's.
    pub rows: Vec<RowLoad>,
    /// Every failure, in order: the rows' (also in [`RowLoad::failures`]) and the walk's own (the
    /// toc or the layer's manifest missing, `Bindings.xml`, the positions bootstrap).
    pub failures: Vec<String>,
    /// Commands `Bindings.xml` registered.
    pub bindings: usize,
}

impl Walk {
    fn absorb(&mut self, rows: Vec<RowLoad>) {
        for row in &rows {
            self.failures.extend(row.failures.iter().cloned());
        }
        self.rows.extend(rows);
    }
}

/// Where the in-game interface's files come from.
pub struct Sources<'a> {
    /// The chain by full path, `\` and `/` alike: the player's install and patch archives.
    pub chain: Read<'a>,
    /// benilla's layer, its flat tree by file name; `None` boots the stock interface alone, as a
    /// dev build's `WOW_STOCK_UI=1`.
    pub layer: Option<Read<'a>>,
}

/// The in-game interface: [`load_core`] (its `Bindings.xml` included), then [`load_layer`] unless
/// `sources.layer` is `None`, then [`bootstrap_positions`]. The caller owns the scope around it:
/// the CVar table ([`crate::script::client_cvars`]), the load's sound suppression and instruction
/// budget, and the addons that follow.
pub fn load_interface(script: &UiScript, sources: &Sources, log: Log) -> Walk {
    let mut walk = load_core(script, sources.chain, log);
    if let Some(layer) = sources.layer {
        let layer = load_layer(script, layer, log);
        walk.rows.extend(layer.rows);
        walk.failures.extend(layer.failures);
    }
    walk.failures.extend(bootstrap_positions(script, log));
    walk
}

/// `toc_text`'s file rows under the toc's own directory (`toc_path`'s), in the toc's order, as the
/// runner builds its paths (`0x6edbe5`, `0x6edcf4`, `0x6edd0c`) and dispatches each in turn
/// (`0x6edd51`).
pub fn toc_rows(toc_path: &str, toc_text: &str) -> Vec<String> {
    let dir = toc_path.rsplit_once('\\').map_or("", |(d, _)| d);
    Toc::parse(toc_text)
        .files
        .iter()
        .map(|f| format!("{dir}\\{f}"))
        .collect()
}

/// [`toc_rows`] of [`FRAMEXML_TOC`].
pub fn core_rows(toc_text: &str) -> Vec<String> {
    toc_rows(FRAMEXML_TOC, toc_text)
}

/// The chain's files as the core reads them.
fn core_files(chain: Read) -> Files {
    Files {
        name: CORE_NAME,
        prefix: "",
        read: chain,
        chunks: Chunks::Path,
        strict: true,
    }
}

/// The core: every row of the chain's `FrameXML.toc`, in its order, each file in document order,
/// then the chain's `Bindings.xml`. A chain without the toc builds no stock interface: the runner
/// reports `"Couldn't open %s"` (`0x846ff4`) at severity 2 (`0x6edc3f`) and returns 0, which
/// `UI_Init` never reads (`0x48fff2`), going on to `Bindings.xml` and the addons. benilla goes on
/// the same way and reports it as a load failure, an `ERROR` line and a row in `/errors`.
///
/// Deviation: no `FrameXML.toc` signature check, where `UI_Init` loads on a failed one and then
/// quits (`0x401560(0xa)` at `0x48ff90`), because the check verifies Blizzard's signature with a key
/// carried in the client binary, which benilla does not ship, and a modified interface file is
/// loaded rather than refused.
pub fn load_core(script: &UiScript, chain: Read, log: Log) -> Walk {
    let mut walk = Walk::default();
    match chain(FRAMEXML_TOC) {
        None => {
            let e = format!("{FRAMEXML_TOC}: not found — the stock interface is not built");
            log(Severity::Error, &format!("ui_script: {e}"));
            script.report_load_failure(&e);
            // Into the UI load's own record, before any banner (`0x6edc3f`).
            let mut record = Status::default();
            record.report(status::FAILURE, status::missing(FRAMEXML_TOC, false));
            script.report_load_status(record);
            walk.failures.push(e);
        }
        Some(bytes) => {
            let rows = core_rows(&crate::source::decode(&bytes));
            let mut toc = Status::default();
            let loaded = load_files_into(script, &core_files(chain), &rows, &mut toc, log);
            let mut record = Status::default();
            toc.close_into(
                &mut record,
                script.framexml_debug(),
                status::toc_banner(FRAMEXML_TOC),
            );
            script.report_load_status(record);
            walk.absorb(loaded);
        }
    }
    match load_core_bindings(script, chain, log) {
        Ok(commands) => walk.bindings = commands,
        Err(e) => walk.failures.push(e),
    }
    walk
}

/// The stock key-binding commands, `UI_Init`'s next step after the toc walk: the chain's
/// [`BINDINGS`] (`0x842f9c`), when the chain has it (`0x48fff9`), through the loader addons' files
/// go through (`0x4b6f70`, called at `0x490018` here and `0x51f443` for an addon). Every command
/// is a row of this file, its body the file's own Lua. The commands registered, 0 without the
/// file, or the parse failure.
pub fn load_core_bindings(script: &UiScript, chain: Read, log: Log) -> Result<usize, String> {
    let Some(bytes) = chain(BINDINGS) else {
        log(
            Severity::Warn,
            &format!(
                "ui_script: {BINDINGS} is not in the patch chain — no key binding has a command"
            ),
        );
        return Ok(0);
    };
    match crate::bindings_xml::parse(&crate::source::decode(&bytes)) {
        Ok(bindings) => {
            script.register_bindings(&bindings);
            log(
                Severity::Info,
                &format!(
                    "bindings: {} commands registered from {BINDINGS}",
                    bindings.len()
                ),
            );
            Ok(bindings.len())
        }
        // `"Couldn't parse XML in %s"` (`0x846fd8`) at severity 2 (`0x4b700c`).
        Err(e) => {
            let e = format!("{BINDINGS}: {e}");
            log(Severity::Error, &format!("ui_script: {e}"));
            script.report_load_failure(&e);
            Err(e)
        }
    }
}

/// A flat tree's file: by its path, then by its basename (an `<Include>` written relative to
/// another folder).
pub fn read_flat(read: Read, req: &str) -> Option<Vec<u8>> {
    let norm = req.replace('\\', "/");
    let base = norm.rsplit('/').next().unwrap_or(&norm).to_string();
    read(&norm).or_else(|| read(&base))
}

/// Every [`LAYER_MANIFEST`] entry, in order, after the whole core: the layer loads as more of the
/// stock load, with no `ADDON_LOADED` and no registry row, as FrameXML does (`0x48ffed`). `layer`
/// is its flat tree by name, read through [`read_flat`].
pub fn load_layer(script: &UiScript, layer: Read, log: Log) -> Walk {
    let mut walk = Walk::default();
    let read = |req: &str| read_flat(layer, req);
    let Some(bytes) = read(LAYER_MANIFEST) else {
        let e = format!("{LAYER_MANIFEST} is not in the shipped UI — it will not load");
        log(Severity::Error, &format!("ui_script: {e}"));
        walk.failures.push(e);
        return walk;
    };
    let rows = Toc::parse(&crate::source::decode(&bytes)).files;
    let files = Files {
        name: LAYER_NAME,
        prefix: "",
        read: &read,
        chunks: Chunks::Addon,
        strict: true,
    };
    walk.absorb(load_files_into(
        script,
        &files,
        &rows,
        &mut Status::default(),
        log,
    ));
    walk
}

/// Runs `UIParent_ManageFramePositions` after the interface has loaded. The buff rows keep
/// `BuffFrame.xml`'s anchors past it: `BuffFrame_OnLoad` never pitches them, and the stock
/// `UIOptionsFrame.lua` does it on `VARIABLES_LOADED` (l.206), after every addon (`0x4900a3`, then
/// `0x4900b2`).
pub fn bootstrap_positions(script: &UiScript, log: Log) -> Vec<String> {
    if let Err(e) = script.run("UIParent_ManageFramePositions()") {
        log(
            Severity::Error,
            &format!("ui_script: managed-positions bootstrap: {e}"),
        );
        return vec![format!("managed-positions bootstrap: {e}")];
    }
    Vec::new()
}

/// Whether a manifest entry is a Lua chunk: by extension, case-insensitively, with `\` a
/// separator like `/` (a `.toc` is written for a Windows client).
pub fn is_lua(entry: &str) -> bool {
    entry
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(entry)
        .rsplit_once('.')
        .is_some_and(|(_, ext)| ext.eq_ignore_ascii_case("lua"))
}

/// The chunk name a `.lua` row of `files` runs under, `path` the row resolved.
pub fn chunk_name(files: &Files, file: &str, path: &str) -> String {
    match files.chunks {
        Chunks::Addon => crate::script::addon_chunk_name(files.name, file),
        Chunks::Path => format!("@{}", path.replace('/', "\\")),
    }
}

/// `rows` of `files`, in order. A manifest lists both kinds of file (the reference's
/// `FrameXML.toc` opens with `GlobalStrings.lua`): a `.lua` runs as a chunk in the shared state,
/// anything else is parsed as FrameXML and materialized. What the reference's `.toc` runner
/// (`0x6edb90`) reports goes into `toc`: a file that did not open or parse, and each document's
/// own record under its banner. Each failure is logged as it happens, retained for the player,
/// and returned on its row, tagged `"<name>/<file>: <error>"`.
pub fn load_files_into(
    script: &UiScript,
    files: &Files,
    rows: &[String],
    toc: &mut Status,
    log: Log,
) -> Vec<RowLoad> {
    let name = files.name;
    let missing = if files.strict {
        Severity::Error
    } else {
        Severity::Warn
    };
    // The `<Include>` / `<Script file=>` provider; `read` is the sandbox.
    let provider = |req: &str| -> Option<Vec<u8>> { (files.read)(req) };
    let mut out = Vec::with_capacity(rows.len());
    for file in rows {
        let started = Instant::now();
        let mut row = RowLoad {
            file: file.clone(),
            frames: 0,
            failures: Vec::new(),
            elapsed: Duration::ZERO,
        };
        // Resolved once into the source's path space, for `read` and the loader alike.
        let path = crate::loader::join_ref(files.prefix, file);
        match (files.read)(&path) {
            None => {
                toc.report(status::FAILURE, status::missing(&path, is_lua(file)));
                let e = format!("{name}/{file}: not found");
                log(missing, &format!("ui_script: {e}"));
                // Retained for the player: the commonest way an addon fails with nothing on screen.
                script.report_load_failure(&e);
                row.failures.push(e);
            }
            // What `<Script file=>` gets: one chunk in the one global state, in manifest order,
            // as `AddOn_Load 0x51f240` hands every listed file to `0x6edb90`.
            Some(bytes) if is_lua(file) => {
                match script.run_chunk_named(&bytes, &chunk_name(files, file, &path)) {
                    Ok(()) => log(Severity::Info, &format!("ui_script: {name}/{file} ran")),
                    Err(err) => {
                        let e = format!("{name}/{file}: {err}");
                        log(Severity::Error, &format!("ui_script: {e}"));
                        // A script error: it reaches the player through the Lua error handler.
                        script.report_script_error(&e, &crate::script::lua_message(&err));
                        row.failures.push(e);
                    }
                }
            }
            Some(bytes) => match crate::framexml::parse(&crate::source::decode(&bytes)) {
                Err(e) => {
                    toc.report(status::FAILURE, status::unparsed(&path));
                    let e = format!("{name}/{file}: {e}");
                    log(Severity::Error, &format!("ui_script: parsing {e}"));
                    // No dialog, as the reference only logs it (FrameXML.log); still retained.
                    script.report_load_failure(&e);
                    row.failures.push(e);
                }
                Ok(doc) => {
                    // The loader resolves relative references against the document's own
                    // directory, and names the file a raise came from.
                    let mut report = crate::loader::load_in(script, &doc, &path, &provider);
                    std::mem::take(&mut report.status).close_into(
                        toc,
                        script.framexml_debug(),
                        status::file_banner(&path),
                    );
                    // `FrameXML_Debug` traces go to the log only: the reference files them at
                    // severity 0 (`0x6ee2bc`) in a per-document record whose surface is untraced.
                    for t in &report.traces {
                        log(Severity::Info, &format!("ui_script({name}/{file}): {t}"));
                    }
                    for w in &report.warnings {
                        log(Severity::Warn, &format!("ui_script({name}/{file}): {w}"));
                        // Retained for `/errors` too, here because only this walk knows the file.
                        script.report_warning(&format!("{name}/{file}: {w}"));
                    }
                    // A file the document names and the provider lacks is a `.toc` line naming no
                    // file: never a script error, as the reference logs `Couldn't open %s` and
                    // carries on, but a failure, so a host's boot tests catch one in a core or
                    // layer document.
                    for m in &report.missing_files {
                        let e = format!("{name}/{file}: {m}");
                        log(missing, &format!("ui_script: {e}"));
                        script.report_load_failure(&e);
                        row.failures.push(e);
                    }
                    for (e, heard) in report.raised() {
                        log(Severity::Error, &format!("ui_script({name}/{file}): {e}"));
                        // A script error, as in the Lua arm; `_ERRORMESSAGE`'s IsVisible guard
                        // shows only a burst's first, as in the reference.
                        let e = format!("{name}/{file}: {e}");
                        script.report_script_error(&e, heard);
                        row.failures.push(e);
                    }
                    log(
                        Severity::Info,
                        &format!(
                            "ui_script: {name}/{file} loaded ({} frames materialized)",
                            report.frames
                        ),
                    );
                    row.frames = report.frames;
                }
            },
        }
        row.elapsed = started.elapsed();
        out.push(row);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    /// An in-memory chain or layer, keyed case-blind with `\`, as the archives answer.
    fn files(entries: &[(&str, &str)]) -> HashMap<String, Vec<u8>> {
        entries
            .iter()
            .map(|(k, v)| (key(k), v.as_bytes().to_vec()))
            .collect()
    }

    fn key(path: &str) -> String {
        path.replace('/', "\\").to_ascii_uppercase()
    }

    /// Loads `chain` and `layer` into a fresh VM; returns the walk, the log and the VM.
    fn load(
        chain: &[(&str, &str)],
        layer: Option<&[(&str, &str)]>,
    ) -> (Walk, Vec<(Severity, String)>, UiScript) {
        let chain = files(chain);
        let layer = layer.map(files);
        let read_chain = |p: &str| chain.get(&key(p)).cloned();
        let read_layer = |p: &str| layer.as_ref().and_then(|l| l.get(&key(p)).cloned());
        let script = UiScript::new().unwrap();
        let sources = Sources {
            chain: &read_chain,
            layer: layer.is_some().then_some(&read_layer as Read),
        };
        let mut lines = Vec::new();
        let walk = load_interface(&script, &sources, &mut |s, l| {
            lines.push((s, l.to_string()))
        });
        (walk, lines, script)
    }

    /// The order the files ran in, from a log each appends to.
    fn ran(script: &UiScript) -> String {
        script.eval::<String>("return LOAD_LOG").unwrap()
    }

    const POSITIONS: (&str, &str) = (
        r"Interface\FrameXML\UIParent.lua",
        "LOAD_LOG = (LOAD_LOG or '') .. 'U'\n\
         function UIParent_ManageFramePositions() LOAD_LOG = LOAD_LOG .. 'P' end",
    );

    /// The walk's order: the core's rows in the toc's order, its `Bindings.xml` (whose bodies run
    /// only when bound keys fire), the layer's rows after every core row, then the positions.
    #[test]
    fn the_core_runs_in_toc_order_then_the_bindings_then_the_layer_then_the_positions() {
        let toc = "## Interface: 11200\n# a comment\nGlobalStrings.lua\nUIParent.lua\nFrames.xml\n";
        let (walk, _, script) = load(
            &[
                (FRAMEXML_TOC, toc),
                (r"Interface\FrameXML\GlobalStrings.lua", "LOAD_LOG = 'G'"),
                POSITIONS,
                (
                    r"Interface\FrameXML\Frames.xml",
                    r#"<Ui><Script file="Frames.lua"/><Frame name="CoreFrame"/></Ui>"#,
                ),
                (
                    r"Interface\FrameXML\Frames.lua",
                    "LOAD_LOG = LOAD_LOG .. 'F'",
                ),
                (
                    BINDINGS,
                    r#"<Bindings><Binding name="MOVEFORWARD">LOAD_LOG = LOAD_LOG .. 'M'</Binding></Bindings>"#,
                ),
            ],
            Some(&[
                (LAYER_MANIFEST, "## Interface: 11200\nFixes.xml\nlate.lua\n"),
                (
                    "Fixes.xml",
                    r#"<Ui><Script>LOAD_LOG = LOAD_LOG .. 'L'</Script></Ui>"#,
                ),
                ("late.lua", "LOAD_LOG = LOAD_LOG .. 'l'"),
            ]),
        );
        assert!(walk.failures.is_empty(), "{:?}", walk.failures);
        assert_eq!(ran(&script), "GUFLlP");
        assert_eq!(walk.bindings, 1);
        let rows: Vec<(&str, usize)> = walk
            .rows
            .iter()
            .map(|r| (r.file.as_str(), r.frames))
            .collect();
        assert_eq!(
            rows,
            [
                (r"Interface\FrameXML\GlobalStrings.lua", 0),
                (r"Interface\FrameXML\UIParent.lua", 0),
                (r"Interface\FrameXML\Frames.xml", 1),
                ("Fixes.xml", 0),
                ("late.lua", 0),
            ]
        );
        assert_eq!(script.widget_kind("CoreFrame"), Some("Frame"));
    }

    #[test]
    fn without_the_layer_only_the_stock_interface_loads() {
        let (walk, _, script) = load(
            &[
                (FRAMEXML_TOC, "UIParent.lua\n"),
                POSITIONS,
                (BINDINGS, "<Bindings/>"),
            ],
            None,
        );
        assert!(walk.failures.is_empty(), "{:?}", walk.failures);
        assert_eq!(ran(&script), "UP");
        assert_eq!(walk.rows.len(), 1);
    }

    /// A row that is missing, will not parse or raises fails alone, tagged with its interface,
    /// and the rows after it still run; a chain without `Bindings.xml` only warns.
    #[test]
    fn failures_are_tagged_and_the_load_goes_on() {
        let (walk, lines, script) = load(
            &[
                (
                    FRAMEXML_TOC,
                    "Gone.lua\nBroken.xml\nRaises.lua\nUIParent.lua\n",
                ),
                (r"Interface\FrameXML\Broken.xml", "<Ui><Frame"),
                (r"Interface\FrameXML\Raises.lua", "error('boom')"),
                POSITIONS,
            ],
            Some(&[(LAYER_MANIFEST, "Missing.xml\n")]),
        );
        let e = &walk.failures;
        assert_eq!(e.len(), 4, "{e:?}");
        assert_eq!(e[0], r"FrameXML/Interface\FrameXML\Gone.lua: not found");
        assert!(
            e[1].starts_with(r"FrameXML/Interface\FrameXML\Broken.xml: "),
            "{e:?}"
        );
        assert!(e[2].starts_with(r"FrameXML/Interface\FrameXML\Raises.lua: "));
        assert!(e[2].contains("boom"), "{e:?}");
        assert_eq!(e[3], "benilla/Missing.xml: not found");
        assert_eq!(ran(&script), "UP", "the rows after a failure still run");
        assert_eq!(walk.rows.iter().map(|r| r.failures.len()).sum::<usize>(), 4);
        assert!(lines
            .iter()
            .any(|(s, l)| *s == Severity::Warn && l.contains(BINDINGS)));
    }

    #[test]
    fn a_missing_toc_builds_no_stock_interface_but_still_loads_the_layer() {
        let (walk, _, script) = load(
            &[],
            Some(&[(LAYER_MANIFEST, "a.lua\n"), ("a.lua", "LOAD_LOG = 'a'")]),
        );
        assert_eq!(walk.failures.len(), 2, "{:?}", walk.failures);
        assert!(walk.failures[0].starts_with(FRAMEXML_TOC));
        assert!(walk.failures[1].starts_with("managed-positions bootstrap"));
        assert_eq!(ran(&script), "a");
    }

    #[test]
    fn a_layer_include_falls_back_to_its_basename() {
        let (walk, _, script) = load(
            &[(FRAMEXML_TOC, "UIParent.lua\n"), POSITIONS],
            Some(&[
                (LAYER_MANIFEST, "Outer.xml\n"),
                (
                    "Outer.xml",
                    r#"<Ui><Include file="..\Elsewhere\Inner.xml"/></Ui>"#,
                ),
                (
                    "Inner.xml",
                    r#"<Ui><Script>LOAD_LOG = LOAD_LOG .. 'I'</Script></Ui>"#,
                ),
            ]),
        );
        assert!(walk.failures.is_empty(), "{:?}", walk.failures);
        assert_eq!(ran(&script), "UIP");
    }

    #[test]
    fn rows_chunk_names_and_kinds_follow_the_client() {
        assert_eq!(
            core_rows("## Title: x\r\nA.lua\r\n# c\r\nB.xml  \r\n"),
            [r"Interface\FrameXML\A.lua", r"Interface\FrameXML\B.xml"]
        );
        let none = |_: &str| None;
        let core = core_files(&none);
        assert_eq!(
            chunk_name(&core, "x", "Interface/FrameXML/A.lua"),
            r"@Interface\FrameXML\A.lua"
        );
        let layer = Files {
            name: LAYER_NAME,
            prefix: "",
            read: &none,
            chunks: Chunks::Addon,
            strict: true,
        };
        assert_eq!(
            chunk_name(&layer, "late.lua", "late.lua"),
            r"@Interface\AddOns\benilla\late.lua"
        );
        assert!(is_lua(r"Interface\FrameXML\A.LUA"));
        assert!(!is_lua("a.lua.xml"));
    }
}
