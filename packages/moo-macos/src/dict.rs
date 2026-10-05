//! Word definitions from the dictionaries macOS ships (Dictionary Services, in-process), parsed
//! into senses. The plain-text entry reads: headword, `| pronunciation |`, then per part of speech
//! numbered senses (`1 … 2 …`, sub-senses after `•`, examples after `:`), then PHRASES,
//! DERIVATIVES, USAGE and ORIGIN sections.
//!
//! The public `DCSCopyTextDefinition` returns only a word's first homograph ("bank" the river
//! side, not "bank" the money one), so the private record functions are looked up at run time
//! for every homograph; without them the first one is still shown.

use std::ffi::{c_char, c_void, CString};
use std::sync::OnceLock;

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::msg_send;
use objc2_foundation::{NSArray, NSString};

type Ptr = *const c_void;

#[repr(C)]
struct CFRange {
    location: isize,
    length: isize,
}

#[link(name = "CoreServices", kind = "framework")]
extern "C" {
    fn DCSCopyTextDefinition(dictionary: Ptr, text: Ptr, range: CFRange) -> Ptr;
}

extern "C" {
    fn dlopen(path: *const c_char, mode: i32) -> *mut c_void;
    fn dlsym(handle: *mut c_void, name: *const c_char) -> *mut c_void;
}

#[derive(Debug, Default, PartialEq)]
pub struct Sense {
    /// "noun", "verb"; empty for entries without one (places, people).
    pub part: String,
    /// Set when this homograph is said differently from the first ("lead" the metal: led).
    pub pronunciation: String,
    pub definition: String,
    /// The first example sentence, if any.
    pub example: String,
}

#[derive(Debug, Default)]
pub struct Entry {
    pub word: String,
    pub pronunciation: String,
    pub senses: Vec<Sense>,
    pub origin: String,
}

/// The default dictionary's plain-text entry for `word` (its first homograph), or None.
pub fn lookup(word: &str) -> Option<String> {
    let word = word.trim();
    if word.is_empty() {
        return None;
    }
    let text = NSString::from_str(word);
    let range = CFRange { location: 0, length: text.length() as isize };
    let raw = unsafe { DCSCopyTextDefinition(std::ptr::null(), Retained::as_ptr(&text).cast(), range) };
    // +1 CFString, toll-free bridged to NSString.
    let def = unsafe { Retained::from_raw(raw as *mut NSString) }?;
    Some(def.to_string())
}

struct Records {
    dictionary: usize,
    search: extern "C" fn(Ptr, Ptr, Ptr, Ptr) -> Ptr,
    headword: extern "C" fn(Ptr) -> Ptr,
    copy_data: extern "C" fn(Ptr, i64) -> Ptr,
}

/// `DCSRecordCopyData` version that returns the same plain text as `DCSCopyTextDefinition`.
const RECORD_TEXT: i64 = 3;
/// The dictionary `DCSCopyTextDefinition` answers from, first one found.
const DICTIONARIES: &[&str] = &["New Oxford American Dictionary", "Oxford Dictionary of English"];

fn records() -> Option<&'static Records> {
    static RECORDS: OnceLock<Option<Records>> = OnceLock::new();
    RECORDS
        .get_or_init(|| unsafe {
            let lib = CString::new("/System/Library/Frameworks/CoreServices.framework/CoreServices").ok()?;
            let h = dlopen(lib.as_ptr(), 1);
            if h.is_null() {
                return None;
            }
            let sym = |n: &str| CString::new(n).ok().map(|c| dlsym(h, c.as_ptr())).filter(|p| !p.is_null());
            let available: extern "C" fn() -> Ptr = std::mem::transmute(sym("DCSCopyAvailableDictionaries")?);
            let name: extern "C" fn(Ptr) -> Ptr = std::mem::transmute(sym("DCSDictionaryGetName")?);
            let r = Records {
                dictionary: 0,
                search: std::mem::transmute(sym("DCSCopyRecordsForSearchString")?),
                headword: std::mem::transmute(sym("DCSRecordGetHeadword")?),
                copy_data: std::mem::transmute(sym("DCSRecordCopyData")?),
            };
            // A +1 NSSet, kept for the life of the process so its dictionaries stay valid.
            let set = available() as *const AnyObject;
            if set.is_null() {
                return None;
            }
            let all: Retained<NSArray<AnyObject>> = msg_send![&*set, allObjects];
            let named: Vec<(String, usize)> = all
                .iter()
                .map(|d| {
                    let p = Retained::as_ptr(&d) as Ptr;
                    (cf_string(name(p)), p as usize)
                })
                .collect();
            let dictionary = DICTIONARIES.iter().find_map(|want| named.iter().find(|(n, _)| n == want).map(|&(_, p)| p))?;
            Some(Records { dictionary, ..r })
        })
        .as_ref()
}

/// A +0 CFString as a Rust string.
fn cf_string(p: Ptr) -> String {
    if p.is_null() {
        return String::new();
    }
    unsafe { &*(p as *const NSString) }.to_string()
}

/// Every homograph of `word` as plain text, the first one first; empty when there is none.
pub fn lookup_all(word: &str) -> Vec<String> {
    let first = lookup(word);
    let Some(r) = records() else {
        return first.into_iter().collect();
    };
    // Records also match other words; keep the homographs of the entry the public call found.
    let base = first.as_deref().map(|t| headword(t.split(" | ").next().unwrap_or(""), word)).unwrap_or_else(|| word.trim().to_string());
    let query = NSString::from_str(word.trim());
    let found = (r.search)(r.dictionary as Ptr, Retained::as_ptr(&query).cast(), std::ptr::null(), std::ptr::null());
    let Some(found) = (unsafe { Retained::from_raw(found as *mut NSArray<AnyObject>) }) else {
        return first.into_iter().collect();
    };
    let mut out = Vec::new();
    for rec in found.iter() {
        let p = Retained::as_ptr(&rec) as Ptr;
        if !cf_string((r.headword)(p)).eq_ignore_ascii_case(&base) {
            continue;
        }
        if let Some(t) = unsafe { Retained::from_raw((r.copy_data)(p, RECORD_TEXT) as *mut NSString) } {
            out.push(t.to_string());
        }
    }
    if out.is_empty() {
        first.into_iter().collect()
    } else {
        out
    }
}

/// The entry for `word` with all its homographs, or None when the dictionary has none, or only
/// one for a different word ("colour" finds "unicolor", whose variants list "-colour").
pub fn define(word: &str) -> Option<Entry> {
    let asked = word.trim();
    let texts = lookup_all(asked);
    let first = texts.first()?;
    if !names_word(first, asked) {
        return None;
    }
    let mut entry = parse(first, asked);
    for t in &texts[1..] {
        let more = parse(t, asked);
        let say = if more.pronunciation != entry.pronunciation { more.pronunciation.clone() } else { String::new() };
        entry.senses.extend(more.senses.into_iter().map(|s| Sense { pronunciation: say.clone(), ..s }));
    }
    (!entry.senses.is_empty()).then_some(entry)
}

/// The entry is for `asked`: its headword, or a form the entry lists ("ran" in run's "(past;
/// ran | ran |)", "children" in "(plural children …)"), or a regular inflection ("notes").
fn names_word(text: &str, asked: &str) -> bool {
    let a = asked.to_lowercase();
    let word = headword(text.split(" | ").next().unwrap_or(""), asked).to_lowercase();
    if word == a || (a.starts_with(&word) && a.len() <= word.len() + 3) {
        return true;
    }
    let head: String = text.chars().take(400).collect::<String>().to_lowercase();
    head.split(|c: char| !(c.is_alphanumeric() || c == '-' || c == '\'' || c == '.')).any(|w| w == a)
}

// Longer names first, so "plural noun" wins over "noun".
const PARTS: &[&str] = &[
    "plural noun", "proper noun", "auxiliary verb", "modal verb", "combining form", "noun", "verb", "adjective",
    "adverb", "pronoun", "preposition", "conjunction", "exclamation", "abbreviation", "determiner",
    "predeterminer", "prefix", "suffix", "symbol", "contraction", "interjection", "article",
];
const SECTIONS: &[&str] = &[" PHRASES ", " PHRASAL VERBS ", " DERIVATIVES ", " USAGE ", " ORIGIN "];

/// One homograph's plain text as an entry.
pub fn parse(text: &str, asked: &str) -> Entry {
    let text = text.trim();
    let (head, pronunciation, body) = match text.find(" | ") {
        Some(i) => {
            let rest = &text[i + 3..];
            match rest.find(" |") {
                Some(j) => (&text[..i], rest[..j].trim(), rest[j + 2..].trim()),
                None => (&text[..i], "", rest.trim()),
            }
        }
        None => (asked, "", text.strip_prefix(asked).unwrap_or(text).trim()),
    };
    let word = headword(head, asked);
    let cut = SECTIONS.iter().filter_map(|s| body.find(s)).min().unwrap_or(body.len());
    let origin = body.find(" ORIGIN ").map(|i| body[i + 8..].trim().to_string()).unwrap_or_default();
    let mut senses = Vec::new();
    let parts = split_parts(&body[..cut]);
    // Text before the first part of speech is a spelling note: "(mainly British English hullo)".
    let named = parts.iter().any(|(p, _)| !p.is_empty());
    for (part, section) in parts {
        if named && part.is_empty() {
            continue;
        }
        for s in numbered(section) {
            let (definition, example) = sense_text(s);
            if !definition.is_empty() {
                senses.push(Sense { part: part.to_string(), pronunciation: String::new(), definition, example });
            }
        }
    }
    Entry { word, pronunciation: pronunciation.to_string(), senses, origin }
}

/// "serendipity ser·en·dip·i·ty" → "serendipity"; "Paris 1 Par·is" → "Paris".
fn headword(head: &str, asked: &str) -> String {
    let words: Vec<&str> = head
        .split_whitespace()
        .take_while(|w| !w.contains('·') && !w.chars().all(|c| c.is_ascii_digit()))
        .collect();
    if words.is_empty() {
        asked.to_string()
    } else {
        words.join(" ")
    }
}

/// Where a part of speech starts at `i`: the body's start or after a sentence, a whole word.
fn part_at(body: &str, i: usize) -> Option<&'static str> {
    let before = body[..i].trim_end();
    if !(before.is_empty() || before.ends_with(['.', '!', '?', ')', '”', '’'])) {
        return None;
    }
    let rest = &body[i..];
    PARTS.iter().copied().find(|p| {
        rest.starts_with(p) && rest[p.len()..].chars().next().is_none_or(|c| c == ' ')
    })
}

fn split_parts(body: &str) -> Vec<(&'static str, &str)> {
    let mut starts: Vec<(usize, &'static str)> = Vec::new();
    for (i, _) in body.char_indices() {
        if (i == 0 || body[..i].ends_with(' ')) && starts.last().is_none_or(|&(s, p)| i >= s + p.len()) {
            if let Some(p) = part_at(body, i) {
                starts.push((i, p));
            }
        }
    }
    if starts.first().is_none_or(|&(s, _)| s > 0) {
        starts.insert(0, (0, ""));
    }
    let mut out = Vec::new();
    for (k, &(s, p)) in starts.iter().enumerate() {
        let end = starts.get(k + 1).map(|&(e, _)| e).unwrap_or(body.len());
        let section = body[s + p.len()..end].trim();
        if !section.is_empty() {
            out.push((p, section));
        }
    }
    out
}

/// "1 a … 2 b …" → ["a …", "b …"]; a section without numbers is one sense. Sense n > 1 starts
/// after a sentence or before a grammar label ("2 [in singular]"), so numbers inside examples
/// ("ran 42 marathons") are not taken for one.
fn numbered(section: &str) -> Vec<&str> {
    let mut marks: Vec<(usize, usize)> = Vec::new();
    let mut n = 1;
    let mut from = 0;
    loop {
        let needle = format!("{n} ");
        let found = section[from..].match_indices(&needle).map(|(i, _)| from + i).find(|&i| {
            let before = section[..i].trim_end();
            let edge = i == 0 || section[..i].ends_with(' ');
            let label = section[i + needle.len()..].starts_with('[');
            edge && (n == 1 || label || before.ends_with(['.', '!', '?', ')', '”', '’']))
        });
        match found {
            Some(i) => {
                marks.push((i, needle.len()));
                from = i + needle.len();
                n += 1;
            }
            None => break,
        }
    }
    if marks.is_empty() {
        return vec![section];
    }
    marks
        .iter()
        .enumerate()
        .map(|(k, &(s, len))| section[s + len..marks.get(k + 1).map(|&(e, _)| e).unwrap_or(section.len())].trim())
        .collect()
}

/// The main sense (before any `•` sub-sense) as (definition, first example), without leading
/// inflections ("(runs)", "(past; ran | ran |)") or grammar labels ("[no object]").
fn sense_text(s: &str) -> (String, String) {
    let t = without_labels(s.split(" • ").next().unwrap_or(""));
    let (def, example) = match example_colon(t) {
        Some(i) => (&t[..i], &t[i + 2..]),
        None => (t, ""),
    };
    let example = first_sentence(without_labels(example.split(" | ").next().unwrap_or("")));
    (first_sentence(def).to_string(), example.to_string())
}

/// Up to the first ". " before a capital, without the final period: drops the notes some senses
/// end with ("…; population 2,203,817 (2006). Paris was held by the Romans…") and keeps "e.g. birth".
fn first_sentence(s: &str) -> &str {
    let mut end = s.len();
    for (i, _) in s.match_indices(". ") {
        if s[i + 2..].chars().next().is_some_and(|c| c.is_uppercase()) {
            end = i;
            break;
        }
    }
    s[..end].trim().trim_end_matches('.').trim()
}

/// The ": " before the examples, outside parentheses ("(Symbol: Pb) a heavy … metal").
fn example_colon(s: &str) -> Option<usize> {
    let mut depth = 0i32;
    for (i, c) in s.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => depth -= 1,
            ':' if depth <= 0 && s[i + 1..].starts_with(' ') => return Some(i),
            _ => {}
        }
    }
    None
}

/// The index of the `)` closing the `(` at index 0.
fn closing_paren(s: &str) -> Option<usize> {
    let mut depth = 0;
    for (i, c) in s.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}

/// Drops leading grammar labels, inflections and the colon after them: "[with object] : …",
/// "(plural hellos) …", "(helloes, helloing, helloed) …". Keeps qualifiers such as "(of a bus,
/// train, or ferry) …".
fn without_labels(s: &str) -> &str {
    let mut t = s.trim();
    loop {
        let before = t;
        if t.starts_with('[') {
            if let Some(e) = t.find(']') {
                t = t[e + 1..].trim_start();
            }
        } else if t.starts_with('(') {
            if let Some(e) = closing_paren(t) {
                if is_inflection(&t[1..e]) {
                    t = t[e + 1..].trim_start();
                }
            }
        }
        t = t.trim_start_matches(':').trim_start();
        if t == before {
            return t;
        }
    }
}

fn is_inflection(inner: &str) -> bool {
    inner.contains('|')
        || inner.contains(';')
        || inner.starts_with(',')
        || ["plural ", "past ", "comparative ", "superlative "].iter().any(|p| inner.starts_with(p))
        || inner.split(',').all(|w| !w.trim().is_empty() && !w.trim().contains(' '))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SERENDIPITY: &str = "serendipity ser·en·dip·i·ty | ˌserənˈdipədē | noun the occurrence and development of events by chance in a happy or beneficial way: a fortunate stroke of serendipity | a series of small serendipities. ORIGIN 1754: coined by Horace Walpole, suggested by The Three Princes of Serendip.";
    const EPHEMERAL: &str = "ephemeral e·phem·er·al | əˈfem(ə)rəl | adjective lasting for a very short time: fashions are ephemeral. • (chiefly of plants) having a very short life cycle: chickweed is an ephemeral weed. noun an ephemeral plant: ephemerals avoid the periods of drought as seeds. DERIVATIVES ephemerality | əˌfem(ə)ˈralədē | noun ORIGIN late 16th century: from Greek ephēmeros.";
    const RUN: &str = "run | rən | verb (runs) (, running | ˈrəniNG |) (past; ran | ran |) (past participle; run | rən |) 1 [no object] move at a speed faster than a walk: the dog ran across the road | she ran the last few yards. • run as a sport or for exercise: I run every morning. • [with object] Dave has run 42 marathons. 2 pass or cause to pass quickly or smoothly in a particular direction: the rumor ran through the pack. 12 [with two objects] North American English (of an object or act) cost (someone) (a specified amount): a new photocopier will run us about $1,300. noun 1 an act or spell of running: I usually go for a run in the morning. 2 a journey accomplished or route taken by a vehicle: the New York-Washington run. PHRASES on the run trying to avoid being captured. ORIGIN Old English rinnan.";
    const PARIS: &str = "Paris 1 Par·is | ˈperəs | 1 the capital of France, on the Seine River; population 2,203,817 (2006). 2 Greek Mythology a Trojan prince.";
    const EG: &str = "e.g. | ˌēˈjē | abbreviation for example: life events (e.g. birth, death and marriage). ORIGIN from Latin exempli gratia ‘for the sake of example’.";

    #[test]
    fn reads_a_one_sense_noun() {
        let e = parse(SERENDIPITY, "serendipity");
        assert_eq!(e.word, "serendipity");
        assert_eq!(e.pronunciation, "ˌserənˈdipədē");
        assert_eq!(e.senses.len(), 1);
        assert_eq!(e.senses[0].part, "noun");
        assert_eq!(e.senses[0].definition, "the occurrence and development of events by chance in a happy or beneficial way");
        assert_eq!(e.senses[0].example, "a fortunate stroke of serendipity");
        assert!(e.origin.starts_with("1754: coined by Horace Walpole"));
    }

    #[test]
    fn splits_parts_of_speech_and_drops_sub_senses() {
        let e = parse(EPHEMERAL, "ephemeral");
        let parts: Vec<&str> = e.senses.iter().map(|s| s.part.as_str()).collect();
        assert_eq!(parts, ["adjective", "noun"]);
        assert_eq!(e.senses[0].definition, "lasting for a very short time");
        assert_eq!(e.senses[1].definition, "an ephemeral plant");
        assert_eq!(e.origin, "late 16th century: from Greek ephēmeros.");
    }

    #[test]
    fn numbers_senses_but_not_numbers_in_examples() {
        let e = parse(RUN, "run");
        assert_eq!(e.word, "run");
        assert_eq!(e.pronunciation, "rən");
        let defs: Vec<(&str, &str)> = e.senses.iter().map(|s| (s.part.as_str(), s.definition.as_str())).collect();
        assert_eq!(
            defs,
            [
                ("verb", "move at a speed faster than a walk"),
                ("verb", "pass or cause to pass quickly or smoothly in a particular direction"),
                ("noun", "an act or spell of running"),
                ("noun", "a journey accomplished or route taken by a vehicle"),
            ]
        );
        assert_eq!(e.senses[0].example, "the dog ran across the road");
    }

    #[test]
    fn reads_entries_without_a_part_of_speech() {
        let e = parse(PARIS, "Paris");
        assert_eq!(e.word, "Paris");
        assert_eq!(e.senses.len(), 2);
        assert_eq!(e.senses[0].part, "");
        assert!(e.senses[0].definition.starts_with("the capital of France"));
        assert_eq!(e.senses[1].definition, "Greek Mythology a Trojan prince");
        let g = parse(EG, "e.g.");
        assert_eq!(g.word, "e.g.");
        assert_eq!(g.senses[0].part, "abbreviation");
        assert_eq!(g.senses[0].definition, "for example");
    }

    #[test]
    fn strips_inflections_with_nested_parentheses() {
        let e = parse("child | CHīld | noun (plural children | ˈCHildr(ə)n |) a young human being below the age of puberty: she'd been playing tennis since she was a child.", "children");
        assert_eq!(e.senses[0].definition, "a young human being below the age of puberty");
        let s = sense_text("(Symbol: Pb) a heavy, bluish-gray, soft, ductile metal, the chemical element of atomic number 82.");
        assert_eq!(s.0, "(Symbol: Pb) a heavy, bluish-gray, soft, ductile metal, the chemical element of atomic number 82");
    }

    #[test]
    fn looks_words_up_in_the_system_dictionary() {
        let e = define("serendipity").expect("the New Oxford American Dictionary ships with macOS");
        assert_eq!(e.word, "serendipity");
        assert_eq!(e.senses[0].part, "noun");
        assert!(define("zzqxv").is_none());
        // The money sense is the second homograph.
        let bank = define("bank").unwrap();
        assert!(bank.senses.iter().any(|s| s.definition.starts_with("a financial establishment")));
        let lead = define("lead").unwrap();
        assert!(lead.senses.iter().any(|s| s.pronunciation == "led" && s.definition.contains("metal")));
        assert_eq!(define("went").unwrap().word, "go");
        assert!(define("colour").is_none_or(|e| e.word == "colour"));
    }
}
