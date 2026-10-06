//! The app's languages: English and Norwegian (bokmål). English text is the
//! key; [`tr`] gives the Norwegian when that's the language in use, and
//! [`trf`] fills `{}` placeholders in a translated template. Anything without
//! a translation (messages from the engine, for one) stays English.
//!
//! The language is per thread: the UI thread sets it, background threads stay
//! English (so text made there is translated when it's shown), and tests can
//! use Norwegian without affecting tests running alongside.

use std::cell::Cell;
use std::fmt::Display;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    English,
    Norwegian,
}

thread_local! {
    static LANG: Cell<Lang> = const { Cell::new(Lang::English) };
}

pub fn set(lang: Lang) {
    LANG.with(|l| l.set(lang));
}

pub fn current() -> Lang {
    LANG.with(Cell::get)
}

/// The language for a setting: `en`, `nb`, or anything else for the
/// system's language.
pub fn resolve(setting: &str) -> Lang {
    match setting {
        "en" => Lang::English,
        "nb" => Lang::Norwegian,
        _ => system(),
    }
}

/// Norwegian when it's the system's preferred language, else English.
pub fn system() -> Lang {
    let preferred = preferred_language();
    if is_norwegian(&preferred) {
        Lang::Norwegian
    } else {
        Lang::English
    }
}

fn is_norwegian(code: &str) -> bool {
    let code = code.trim().trim_matches('"').to_ascii_lowercase();
    ["nb", "no", "nn"].iter().any(|p| {
        code == *p || code.starts_with(&format!("{p}-")) || code.starts_with(&format!("{p}_"))
    })
}

#[cfg(target_os = "macos")]
fn preferred_language() -> String {
    // `defaults read -g AppleLanguages` → ( "nb-NO", "en-US" )
    std::process::Command::new("defaults")
        .args(["read", "-g", "AppleLanguages"])
        .output()
        .ok()
        .and_then(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .map(|l| l.trim().trim_end_matches(','))
                .find(|l| l.starts_with('"') || l.chars().next().is_some_and(char::is_alphabetic))
                .map(str::to_string)
        })
        .unwrap_or_default()
}

#[cfg(not(target_os = "macos"))]
fn preferred_language() -> String {
    ["LANGUAGE", "LC_ALL", "LC_MESSAGES", "LANG"]
        .iter()
        .filter_map(|k| std::env::var(k).ok())
        .find(|v| !v.is_empty())
        .unwrap_or_default()
}

/// `s` in the current language.
pub fn tr(s: &str) -> &str {
    tr_in(current(), s)
}

fn tr_in(lang: Lang, s: &str) -> &str {
    match lang {
        Lang::English => s,
        Lang::Norwegian => nb(s).unwrap_or(s),
    }
}

/// A translated template with each `{}` replaced by the next argument.
pub fn trf(template: &str, args: &[&dyn Display]) -> String {
    trf_in(current(), template, args)
}

fn trf_in(lang: Lang, template: &str, args: &[&dyn Display]) -> String {
    let mut out = String::new();
    let mut args = args.iter();
    let mut rest = tr_in(lang, template);
    while let Some(i) = rest.find("{}") {
        out.push_str(&rest[..i]);
        if let Some(arg) = args.next() {
            out.push_str(&arg.to_string());
        }
        rest = &rest[i + 2..];
    }
    out.push_str(rest);
    out
}

/// "just now", "5 min ago", … in the current language.
pub fn ago(now_ms: i64, then_ms: i64) -> String {
    ago_in(current(), now_ms, then_ms)
}

fn ago_in(lang: Lang, now_ms: i64, then_ms: i64) -> String {
    let secs = (now_ms - then_ms).max(0) / 1000;
    let f = |t: &str, n: i64| trf_in(lang, t, &[&n]);
    match secs {
        0..60 => tr_in(lang, "just now").to_string(),
        60..3600 => f("{} min ago", secs / 60),
        3600..86_400 => f("{} h ago", secs / 3600),
        86_400..172_800 => tr_in(lang, "yesterday").to_string(),
        _ if secs < 60 * 86_400 => f("{} days ago", secs / 86_400),
        _ => f("{} months ago", secs / (30 * 86_400)),
    }
}

fn nb(s: &str) -> Option<&'static str> {
    Some(match s {
        // Navigation and the Optimise screen.
        "WORKSPACE" => "ARBEIDSOMRÅDE",
        "SETTINGS" => "INNSTILLINGER",
        "SUPPORT" => "STØTTE",
        "RESULTS" => "RESULTATER",
        "Optimise" => "Optimaliser",
        "History" => "Historikk",
        "Crop image…" => "Beskjær bilde…",
        "Preferences" => "Innstillinger",
        "About" => "Om",
        "{} working…" => "{} arbeider…",
        "Make images, video, PDF and audio smaller." => "Gjør bilder, video, PDF og lyd mindre.",
        "Drop files here" => "Slipp filer her",
        "images · video · PDF · audio" => "bilder · video · PDF · lyd",
        "{} clipboard" => "{} utklippstavle",
        "{} show" => "{} vis",
        "  Open files…  " => "  Åpne filer…  ",
        "Optimise clipboard" => "Optimaliser utklippstavlen",
        "Clear" => "Tøm",
        "Compression" => "Komprimering",
        "images: quality target “{}”" => "bilder: kvalitetsmål «{}»",
        "Aggressive" => "Aggressiv",
        "Convert to" => "Konverter til",
        "Pipeline" => "Pipeline",
        "Keep format" => "Behold format",
        "Keep format — just optimise" => "Behold format — bare optimaliser",
        "Images are saved as {} next to the originals{}" => {
            "Bilder lagres som {} ved siden av originalene{}"
        }
        " · transparent areas become white" => " · gjennomsiktige områder blir hvite",
        " · limited to 256 colours" => " · begrenset til 256 farger",
        " · uncompressed, large files" => " · ukomprimert, store filer",
        " · lossless" => " · tapsfritt",
        "Clipboard" => "Utklippstavle",
        "Invalid pipeline" => "Ugyldig pipeline",
        "{}  ·  already optimised — skipped" => "{}  ·  allerede optimalisert — hoppet over",
        "  ·  aggressive" => "  ·  aggressiv",
        "  ·  copied back" => "  ·  kopiert tilbake",
        "Image" => "Bilde",
        "Convert to another format" => "Konverter til et annet format",
        "Reveal" => "Vis",
        "Copy" => "Kopier",
        "Crop…" => "Beskjær…",
        "Show in Finder" => "Vis i Finder",
        "Saved next to it; the original is kept." => "Lagres ved siden av; originalen beholdes.",
        "Small, transparency, works in all browsers" => {
            "Liten, gjennomsiktighet, virker i alle nettlesere"
        }
        "Smallest, modern browsers and apps" => "Minst, moderne nettlesere og apper",
        "Apple Photos format, small" => "Formatet til Apple Bilder, lite",
        "Next-gen, limited support" => "Neste generasjon, begrenset støtte",
        "Lossless, transparency" => "Tapsfritt, gjennomsiktighet",
        "Photos, opens everywhere, no transparency" => {
            "Bilder, åpnes overalt, uten gjennomsiktighet"
        }
        "256 colours, opens everywhere" => "256 farger, åpnes overalt",
        "Lossless, print and archiving" => "Tapsfritt, trykk og arkiv",
        "Uncompressed, legacy apps" => "Ukomprimert, eldre apper",
        // Crop.
        "Crop" => "Beskjær",
        "Cancel" => "Avbryt",
        "Apply crop" => "Beskjær",
        "Drag to select a region." => "Dra for å velge et område.",
        // Updates and About.
        "Downloading update…" => "Laster ned oppdatering…",
        "Installing update…" => "Installerer oppdatering…",
        "Restarting…" => "Starter på nytt…",
        "Update failed: {}" => "Oppdateringen feilet: {}",
        "Updating…" => "Oppdaterer…",
        "Checking for updates…" => "Ser etter oppdateringer…",
        "Update available — v{}" => "Oppdatering tilgjengelig — v{}",
        "Update & Restart" => "Oppdater og start på nytt",
        "Download" => "Last ned",
        "You're on the latest version" => "Du har nyeste versjon",
        "Check for updates" => "Se etter oppdateringer",
        "Dismiss" => "Lukk",
        "Version {}" => "Versjon {}",
        "Make your media smaller — images, video, PDF and audio." => {
            "Gjør mediene dine mindre — bilder, video, PDF og lyd."
        }
        "Website · kwhorne.com" => "Nettsted · kwhorne.com",
        "Developed by Knut W. Horne" => "Utviklet av Knut W. Horne",
        "Show the welcome tour" => "Vis velkomstturen",
        // Menu bar.
        "Open xpress" => "Åpne xpress",
        "Collect clips" => "Samle klipp",
        "Quit xpress" => "Avslutt xpress",
        "Clipboard history" => "Utklippshistorikk",
        // Preferences.
        "Defaults applied to every optimisation. Changes are saved automatically." => {
            "Standardvalg for all optimalisering. Endringer lagres automatisk."
        }
        "Language" => "Språk",
        "The app's language" => "Språket i appen",
        "System" => "System",
        "Keep a backup" => "Behold sikkerhetskopi",
        "Save the original as .name.orig" => "Lagre originalen som .navn.orig",
        "Strip metadata" => "Fjern metadata",
        "Remove EXIF (camera, location, date)" => "Fjern EXIF (kamera, sted, dato)",
        "Remove location" => "Fjern sted",
        "Drop GPS / where it was taken, keep the rest" => {
            "Fjern GPS / hvor det ble tatt, behold resten"
        }
        "Quality target" => "Kvalitetsmål",
        "Images: the smallest file that still looks this good" => {
            "Bilder: den minste filen som fortsatt ser så bra ut"
        }
        "Off — use the compression slider" => "Av — bruk komprimeringen",
        "Visually lossless" => "Visuelt tapsfritt",
        "High" => "Høy",
        "Medium" => "Middels",
        "Low" => "Lav",
        "Skip already-optimised files" => "Hopp over filer som allerede er optimalisert",
        "Leave files xpress already squeezed with these settings" => {
            "La filer xpress alt har komprimert med disse innstillingene være"
        }
        "Aggressive by default" => "Aggressiv som standard",
        "Trade a little quality for smaller files" => "Bytt litt kvalitet mot mindre filer",
        "Open at login" => "Åpne ved pålogging",
        "Start xpress in the menu bar when you log in" => {
            "Start xpress i menylinjen når du logger på"
        }
        "Float on top" => "Alltid øverst",
        "Keep the window above others" => "Hold vinduet over andre vinduer",
        "Default pipeline" => "Standard-pipeline",
        "Runs when “Pipeline” is enabled on the Optimise screen." => {
            "Kjøres når «Pipeline» er slått på under Optimaliser."
        }
        "Keep what you copy, searchable under History" => {
            "Ta vare på det du kopierer, søkbart under Historikk"
        }
        "Keep what you copy, searchable under History ({})" => {
            "Ta vare på det du kopierer, søkbart under Historikk ({})"
        }
        "Include screenshots" => "Ta med skjermbilder",
        "Add new screenshots to the history" => "Legg nye skjermbilder i historikken",
        "Find text in images" => "Finn tekst i bilder",
        "Recognise words in screenshots and images, on this Mac" => {
            "Gjenkjenn ord i skjermbilder og bilder, på denne Macen"
        }
        "Paste directly" => "Lim inn direkte",
        "After you choose a clip, paste it into the app you were using" => {
            "Når du velger et klipp, limes det inn i appen du brukte"
        }
        "Needs permission: System Settings → Privacy & Security → Accessibility → xpress" => {
            "Trenger tillatelse: Systeminnstillinger → Personvern og sikkerhet → Tilgjengelighet → xpress"
        }
        "Open Settings" => "Åpne Innstillinger",
        "Sync with iCloud" => "Synk med iCloud",
        "Share the history between your Macs through iCloud Drive" => {
            "Del historikken mellom Macene dine via iCloud Drive"
        }
        "Syncing…" => "Synker…",
        "Synced {}" => "Synket {}",
        " — no other Mac yet" => " — ingen annen Mac ennå",
        " with {}" => " med {}",
        " with {} Macs" => " med {} Macer",
        " · {} changes waiting for iCloud" => " · {} endringer venter på iCloud",
        "iCloud Drive is off — turn it on in System Settings → Apple Account → iCloud." => {
            "iCloud Drive er av — slå det på i Systeminnstillinger → Apple-konto → iCloud."
        }
        "Ignore apps" => "Ignorer apper",
        "Don't record what you copy in these apps — password managers are always left out" => {
            "Ikke registrer det du kopierer i disse appene — passordbehandlere er alltid utelatt"
        }
        "Add app…" => "Legg til app…",
        "Choose from Applications…" => "Velg fra Programmer…",
        "Record it again" => "Registrer den igjen",
        "Delete its 1 clip" => "Slett klippet",
        "Delete its {} clips" => "Slett {} klipp",
        "Click again to delete" => "Klikk igjen for å slette",
        "Keep history" => "Behold historikk",
        "Pinned clips are always kept" => "Festede klipp beholdes alltid",
        "1 day" => "1 dag",
        "1 week" => "1 uke",
        "1 month" => "1 måned",
        "3 months" => "3 måneder",
        "1 year" => "1 år",
        "Forever" => "For alltid",
        "Clear history" => "Tøm historikken",
        "{} clips · {} · pinned clips stay" => "{} klipp · {} · festede klipp blir",
        "Clear…" => "Tøm…",
        "Click again to clear" => "Klikk igjen for å tømme",
        "Shortcuts" => "Hurtigtaster",
        "Work in every app. Click one to change it." => {
            "Virker i alle apper. Klikk på en for å endre den."
        }
        "Optimise the image you copied" => "Optimaliser bildet du kopierte",
        "Bring the window to the front" => "Hent vinduet fram",
        "Search what you copied and paste it again" => {
            "Søk i det du har kopiert, og lim det inn igjen"
        }
        "Press keys…" => "Trykk tastene…",
        "Click, then press the new shortcut" => "Klikk, og trykk den nye hurtigtasten",
        "Reset" => "Tilbakestill",
        "Press the new shortcut · ⌫ turns it off · esc cancels" => {
            "Trykk den nye hurtigtasten · ⌫ slår den av · esc avbryter"
        }
        "Off" => "Av",
        "Space" => "Mellomrom",
        "Use ⌘, ⌃ or ⌥ with the key." => "Bruk ⌘, ⌃ eller ⌥ sammen med tasten.",
        "That key can't be a shortcut." => "Den tasten kan ikke være en hurtigtast.",
        "Already used for “{}”." => "Allerede brukt til «{}».",
        "{} couldn't be set up — another app may be using it." => {
            "{} kunne ikke settes opp — en annen app bruker den kanskje."
        }
        "Opening at login needs macOS 13 or later and the installed app." => {
            "Åpning ved pålogging krever macOS 13 eller nyere og den installerte appen."
        }
        // Welcome.
        "Skip" => "Hopp over",
        "← Back" => "← Tilbake",
        "Next →" => "Neste →",
        "Start using xpress" => "Begynn å bruke xpress",
        "Welcome to xpress" => "Velkommen til xpress",
        "Make images, video, PDFs and audio smaller — without them looking or sounding worse." => {
            "Gjør bilder, video, PDF-er og lyd mindre — uten at de ser eller høres dårligere ut."
        }
        "Drop files onto the window" => "Slipp filer i vinduet",
        "They come out smaller; the originals are kept as backups." => {
            "De blir mindre; originalene beholdes som sikkerhetskopier."
        }
        "Lives in the menu bar" => "Bor i menylinjen",
        " or press {}" => " eller trykk {}",
        "Click the xpress icon in the menu bar{}. Closing the window keeps it running." => {
            "Klikk på xpress-ikonet i menylinjen{}. Lukker du vinduet, fortsetter den å kjøre."
        }
        "Copy large, paste small" => "Kopier stort, lim inn smått",
        "Copy an image and press {}: the smaller one is ready to paste." => {
            "Kopier et bilde og trykk {}: det mindre bildet er klart til å limes inn."
        }
        "Copy an image and choose Optimise clipboard: the smaller one is ready to paste." => {
            "Kopier et bilde og velg Optimaliser utklippstavlen: det mindre bildet er klart til å limes inn."
        }
        "Your clipboard, remembered" => "Utklippstavlen din, husket",
        "Keep everything you copy and every screenshot, and find it again{} — also by the words inside images. It stays on this Mac, and passwords are never saved." => {
            "Ta vare på alt du kopierer og alle skjermbilder, og finn dem igjen{} — også etter ordene inne i bilder. Alt blir på denne Macen, og passord lagres aldri."
        }
        "Record what you copy" => "Registrer det du kopierer",
        "Paste a chosen clip into the app you were using (macOS asks for permission)" => {
            "Lim inn et valgt klipp i appen du brukte (macOS ber om tillatelse)"
        }
        "More about the history" => "Mer om historikken",
        "Ready to go" => "Klar",
        "A few last things — all of them can be changed in Preferences." => {
            "Noen siste ting — alt kan endres i Innstillinger."
        }
        "Command line" => "Kommandolinje",
        "Scripts, folders and CI: brew install kwhorne/tap/xpress" => {
            "Skript, mapper og CI: brew install kwhorne/tap/xpress"
        }
        "Learn more" => "Les mer",
        // History.
        "Everything you copy and every screenshot — search, then copy it back." => {
            "Alt du kopierer og alle skjermbilder — søk, og kopier det tilbake."
        }
        "⧉ Collect" => "⧉ Samle",
        "Put everything you copy from now on into one multi-clip, to paste all of it at once" => {
            "Legg alt du kopierer fra nå av i ett multiklipp, så du kan lime inn alt på én gang"
        }
        "Clipboard history is off" => "Utklippshistorikken er av",
        "Turn it on to keep what you copy and your screenshots, and find them again — also by the text inside images. Everything stays on this Mac; passwords and other private clipboard content are never saved." => {
            "Slå den på for å ta vare på det du kopierer og skjermbildene dine, og finne dem igjen — også etter teksten inne i bilder. Alt blir på denne Macen; passord og annet privat innhold lagres aldri."
        }
        "Turn on clipboard history" => "Slå på utklippshistorikk",
        "Collecting — everything you copy goes into one multi-clip." => {
            "Samler — alt du kopierer havner i ett multiklipp."
        }
        "Done" => "Ferdig",
        "Multi-clip · {} items" => "Multiklipp · {} elementer",
        "Copy all" => "Kopier alt",
        "Nothing yet — copy something or take a screenshot." => {
            "Ingenting ennå — kopier noe eller ta et skjermbilde."
        }
        "No clips match." => "Ingen klipp passer.",
        "{} clips · {}   ↑↓ choose · ⏎ copy · ⌘1–9 · ⌘-click to pick several · esc" => {
            "{} klipp · {}   ↑↓ velg · ⏎ kopier · ⌘1–9 · ⌘-klikk for å velge flere · esc"
        }
        "Search text, links, files and words in images…" => {
            "Søk i tekst, lenker, filer og ord i bilder…"
        }
        "All apps" => "Alle apper",
        "All" => "Alle",
        "Pinned" => "Festet",
        "Edit…" => "Rediger…",
        "Delete category" => "Slett kategori",
        "+ Category" => "+ Kategori",
        "Group clips by hand, or automatically by app, kind or words" => {
            "Grupper klipp for hånd, eller automatisk etter app, type eller ord"
        }
        "{} selected" => "{} valgt",
        "Copy together" => "Kopier samlet",
        "Combine into a multi-clip and copy it (⏎)" => {
            "Slå sammen til et multiklipp og kopier det (⏎)"
        }
        "Combine" => "Slå sammen",
        "Keep them together as one multi-clip" => "Hold dem samlet som ett multiklipp",
        "Add to category" => "Legg i kategori",
        "Pin" => "Fest",
        "Unpin" => "Løsne",
        "Delete" => "Slett",
        "Clear selection" => "Fjern utvalget",
        "New category" => "Ny kategori",
        "Edit category" => "Rediger kategori",
        "Name, e.g. Receipts" => "Navn, f.eks. Kvitteringer",
        "Add clips automatically" => "Legg til klipp automatisk",
        "New clips that match everything set here join by themselves." => {
            "Nye klipp som passer med alt som er satt her, legges til av seg selv."
        }
        "Copied in" => "Kopiert i",
        "Any app" => "Hvilken som helst app",
        "Kind" => "Type",
        "Any kind" => "Hvilken som helst type",
        "Containing" => "Som inneholder",
        "words, also in images" => "ord, også i bilder",
        "Save" => "Lagre",
        "Save to history" => "Lagre i historikken",
        "Keep it as a new clip" => "Behold det som et nytt klipp",
        "Close" => "Lukk",
        "Copy to the clipboard" => "Kopier til utklippstavlen",
        "Copy to the clipboard (⌘{})" => "Kopier til utklippstavlen (⌘{})",
        "Pin — keep forever" => "Fest — behold for alltid",
        "Show items" => "Vis innholdet",
        "Copy text in image" => "Kopier teksten i bildet",
        "Open link" => "Åpne lenke",
        "Don't record from {}" => "Ikke registrer fra {}",
        "Add it to Preferences → Ignore apps" => "Legg den til i Innstillinger → Ignorer apper",
        "Categories" => "Kategorier",
        "New category…" => "Ny kategori…",
        "There's already a category called “{}”." => {
            "Det finnes allerede en kategori som heter «{}»."
        }
        "Multi-clip  ·  {}  ·  {} items" => "Multiklipp  ·  {}  ·  {} elementer",
        "{} items" => "{} elementer",
        "Multi-clip" => "Multiklipp",
        "Text" => "Tekst",
        "Link" => "Lenke",
        "Code" => "Kode",
        "Colour" => "Farge",
        "Screenshot" => "Skjermbilde",
        "Files" => "Filer",
        "Links" => "Lenker",
        "Colours" => "Farger",
        "Images" => "Bilder",
        "Screenshots" => "Skjermbilder",
        "Multi-clips" => "Multiklipp",
        "just now" => "akkurat nå",
        "{} min ago" => "for {} min siden",
        "{} h ago" => "for {} t siden",
        "yesterday" => "i går",
        "{} days ago" => "for {} dager siden",
        "{} months ago" => "for {} måneder siden",
        // Apple Intelligence.
        "Apple Intelligence · {}" => "Apple Intelligence · {}",
        "Summarise" => "Oppsummer",
        "Proofread" => "Korrekturles",
        "Make shorter" => "Gjør kortere",
        "Make professional" => "Gjør profesjonell",
        "Make friendly" => "Gjør vennlig",
        "Translate to English" => "Oversett til engelsk",
        "Summary" => "Sammendrag",
        "Proofread text" => "Korrekturlest tekst",
        "Translation" => "Oversettelse",
        "Rewritten" => "Omskrevet",
        "Turn on Apple Intelligence in System Settings → Apple Intelligence & Siri." => {
            "Slå på Apple Intelligence i Systeminnstillinger → Apple Intelligence og Siri."
        }
        "This Mac doesn't support Apple Intelligence." => {
            "Denne Macen støtter ikke Apple Intelligence."
        }
        "Apple Intelligence is still getting ready — try again in a while." => {
            "Apple Intelligence gjør seg fortsatt klar — prøv igjen om litt."
        }
        "Apple Intelligence isn't available right now." => {
            "Apple Intelligence er ikke tilgjengelig nå."
        }
        "Apple Intelligence needs macOS 26 or later." => {
            "Apple Intelligence krever macOS 26 eller nyere."
        }
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // The language is passed in: other tests run in parallel and must see
    // English, so nothing here changes the global setting.
    #[test]
    fn translating() {
        use Lang::{English as En, Norwegian as Nb};
        assert!(is_norwegian("\"nb-NO\""));
        assert!(is_norwegian("nn_NO.UTF-8"));
        assert!(is_norwegian("no"));
        assert!(!is_norwegian("en-US"));
        assert!(!is_norwegian("nl-NL"));
        assert_eq!(resolve("en"), En);
        assert_eq!(resolve("nb"), Nb);

        assert_eq!(tr_in(En, "Drop files here"), "Drop files here");
        assert_eq!(trf_in(En, "{} selected", &[&3]), "3 selected");
        assert_eq!(ago_in(En, 120_000, 0), "2 min ago");

        assert_eq!(tr_in(Nb, "Drop files here"), "Slipp filer her");
        assert_eq!(tr_in(Nb, "not in the table"), "not in the table");
        assert_eq!(trf_in(Nb, "{} selected", &[&3]), "3 valgt");
        assert_eq!(
            trf_in(
                Nb,
                "Images are saved as {} next to the originals{}",
                &[&"WebP", &""]
            ),
            "Bilder lagres som WebP ved siden av originalene"
        );
        assert_eq!(ago_in(Nb, 0, 0), "akkurat nå");
        assert_eq!(ago_in(Nb, 3 * 86_400_000, 0), "for 3 dager siden");
        assert_eq!(current(), En, "untouched");
    }

    #[test]
    fn every_template_keeps_its_placeholders() {
        let source = include_str!("i18n.rs");
        let table = &source[source.find("fn nb(").unwrap()..source.find("#[cfg(test)]").unwrap()];
        for line in table.lines().filter(|l| l.contains("\" => ")) {
            let (en, nb) = line.split_once("\" => ").unwrap();
            if !nb.contains('"') {
                continue; // a long translation on the next line
            }
            assert_eq!(en.matches("{}").count(), nb.matches("{}").count(), "{line}");
        }
    }
}
