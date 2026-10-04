//! Contacts: name search over the address book (Contacts framework). Reading needs the user's
//! permission; only `request` shows the prompt, everything else fails quietly without access.

use std::ptr::NonNull;

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject, Bool};
use objc2::msg_send;
use objc2_foundation::{NSArray, NSError, NSString};

#[link(name = "Contacts", kind = "framework")]
extern "C" {
    static CNContactGivenNameKey: &'static NSString;
    static CNContactFamilyNameKey: &'static NSString;
    static CNContactNicknameKey: &'static NSString;
    static CNContactOrganizationNameKey: &'static NSString;
    static CNContactJobTitleKey: &'static NSString;
    static CNContactEmailAddressesKey: &'static NSString;
    static CNContactPhoneNumbersKey: &'static NSString;
}

/// macOS kills a bare binary that asks for Contacts without a usage description, and only
/// Nimble.app has an Info.plist file; this section gives the unbundled binary one. No bundle id:
/// that would change how other permissions and notifications see the dev build.
#[used]
#[link_section = "__TEXT,__info_plist"]
static INFO_PLIST: [u8; include_bytes!("info.plist").len()] = *include_bytes!("info.plist");

/// `CNEntityTypeContacts`.
const ENTITY_CONTACTS: isize = 0;
/// `CNContactSortOrderUserDefault`.
const SORT_USER_DEFAULT: isize = 1;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Field {
    pub label: String,
    pub value: String,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Contact {
    pub id: String,
    pub given: String,
    pub family: String,
    pub nickname: String,
    pub org: String,
    pub title: String,
    pub emails: Vec<Field>,
    pub phones: Vec<Field>,
}

impl Contact {
    /// "Given Family", else the nickname, company, first email or first phone.
    pub fn name(&self) -> String {
        let full = [self.given.trim(), self.family.trim()].iter().filter(|p| !p.is_empty()).copied().collect::<Vec<_>>().join(" ");
        let named = [full.as_str(), self.nickname.trim(), self.org.trim()].into_iter().find(|n| !n.is_empty()).map(str::to_string);
        named
            .or_else(|| self.emails.first().map(|e| e.value.clone()))
            .or_else(|| self.phones.first().map(|p| p.value.clone()))
            .unwrap_or_default()
    }
}

/// notDetermined, restricted, denied, authorized or limited. Never prompts.
pub fn status() -> &'static str {
    let Some(cls) = AnyClass::get(c"CNContactStore") else { return "restricted" };
    let s: isize = unsafe { msg_send![cls, authorizationStatusForEntityType: ENTITY_CONTACTS] };
    match s {
        0 => "notDetermined",
        1 => "restricted",
        2 => "denied",
        3 => "authorized",
        _ => "limited",
    }
}

pub fn allowed() -> bool {
    matches!(status(), "authorized" | "limited")
}

/// Shows the system prompt (once; later calls answer from the saved choice). `done(granted)` runs
/// on a framework queue.
pub fn request(done: impl Fn(bool) + 'static) {
    let Some(cls) = AnyClass::get(c"CNContactStore") else { return done(false) };
    let store: Retained<AnyObject> = unsafe { msg_send![cls, new] };
    let keep = store.clone();
    let block = RcBlock::new(move |granted: Bool, _e: *mut NSError| {
        let _ = &keep;
        done(granted.as_bool());
    });
    unsafe {
        let _: () = msg_send![&*store, requestAccessForEntityType: ENTITY_CONTACTS, completionHandler: &*block];
    }
}

fn keys() -> Retained<NSArray<NSString>> {
    unsafe {
        NSArray::from_slice(&[
            CNContactGivenNameKey,
            CNContactFamilyNameKey,
            CNContactNicknameKey,
            CNContactOrganizationNameKey,
            CNContactJobTitleKey,
            CNContactEmailAddressesKey,
            CNContactPhoneNumbersKey,
        ])
    }
}

fn text(s: Option<Retained<NSString>>) -> String {
    s.map(|s| s.to_string()).unwrap_or_default()
}

/// "_$!<Mobile>!$_" and friends, in the user's language.
fn label(lv: &AnyObject) -> String {
    let raw: Option<Retained<NSString>> = unsafe { msg_send![lv, label] };
    let Some(raw) = raw else { return String::new() };
    let Some(cls) = AnyClass::get(c"CNLabeledValue") else { return raw.to_string() };
    let shown: Option<Retained<NSString>> = unsafe { msg_send![cls, localizedStringForLabel: &*raw] };
    shown.map(|s| s.to_string()).unwrap_or_else(|| raw.to_string())
}

fn fields(contact: &AnyObject, phones: bool) -> Vec<Field> {
    let list: Option<Retained<NSArray<AnyObject>>> =
        unsafe { if phones { msg_send![contact, phoneNumbers] } else { msg_send![contact, emailAddresses] } };
    let Some(list) = list else { return Vec::new() };
    list.iter()
        .filter_map(|lv| {
            let value: Option<Retained<AnyObject>> = unsafe { msg_send![&*lv, value] };
            let value = value?;
            let value = if phones {
                text(unsafe { msg_send![&*value, stringValue] })
            } else {
                let s: &NSString = unsafe { &*(Retained::as_ptr(&value) as *const NSString) };
                s.to_string()
            };
            (!value.is_empty()).then(|| Field { label: label(&lv), value })
        })
        .collect()
}

fn read(c: &AnyObject) -> Contact {
    unsafe {
        Contact {
            id: text(msg_send![c, identifier]),
            given: text(msg_send![c, givenName]),
            family: text(msg_send![c, familyName]),
            nickname: text(msg_send![c, nickname]),
            org: text(msg_send![c, organizationName]),
            title: text(msg_send![c, jobTitle]),
            emails: fields(c, false),
            phones: fields(c, true),
        }
    }
}

/// 0 when the name starts with `query`, 1 when a word in it does, 2 otherwise.
fn rank(c: &Contact, query: &str) -> u8 {
    let q = query.trim().to_lowercase();
    let name = c.name().to_lowercase();
    if q.is_empty() || name.starts_with(&q) {
        0
    } else if name.split_whitespace().any(|w| w.starts_with(&q)) {
        1
    } else {
        2
    }
}

pub fn sort(list: &mut [Contact], query: &str) {
    list.sort_by_cached_key(|c| (rank(c, query), c.name().to_lowercase()));
}

/// Contacts whose name matches `query`, best first; every contact (address-book order) when the
/// query is empty.
pub fn search(query: &str, limit: usize) -> Result<Vec<Contact>, String> {
    if !allowed() {
        return Err(format!("no access to contacts ({})", status()));
    }
    let store_cls = AnyClass::get(c"CNContactStore").ok_or("Contacts is unavailable")?;
    let store: Retained<AnyObject> = unsafe { msg_send![store_cls, new] };
    let keys = keys();
    if query.trim().is_empty() {
        return all(&store, &keys, limit);
    }
    let contact_cls = AnyClass::get(c"CNContact").ok_or("Contacts is unavailable")?;
    let name = NSString::from_str(query.trim());
    let predicate: Retained<AnyObject> = unsafe { msg_send![contact_cls, predicateForContactsMatchingName: &*name] };
    let found: Result<Retained<NSArray<AnyObject>>, Retained<NSError>> =
        unsafe { msg_send![&*store, unifiedContactsMatchingPredicate: &*predicate, keysToFetch: &*keys, error: _] };
    let found = found.map_err(|e| e.localizedDescription().to_string())?;
    let mut list: Vec<Contact> = found.iter().map(|c| read(&c)).collect();
    sort(&mut list, query);
    list.truncate(limit);
    Ok(list)
}

fn all(store: &AnyObject, keys: &NSArray<NSString>, limit: usize) -> Result<Vec<Contact>, String> {
    let req_cls = AnyClass::get(c"CNContactFetchRequest").ok_or("Contacts is unavailable")?;
    let req: Retained<AnyObject> = unsafe {
        let r: objc2::rc::Allocated<AnyObject> = msg_send![req_cls, alloc];
        msg_send![r, initWithKeysToFetch: keys]
    };
    unsafe {
        let _: () = msg_send![&*req, setSortOrder: SORT_USER_DEFAULT];
        let _: () = msg_send![&*req, setUnifyResults: true];
    }
    let out = std::cell::RefCell::new(Vec::new());
    let block = RcBlock::new(|c: NonNull<AnyObject>, stop: NonNull<Bool>| {
        let mut list = out.borrow_mut();
        list.push(read(unsafe { c.as_ref() }));
        if list.len() >= limit {
            unsafe { *stop.as_ptr() = Bool::YES };
        }
    });
    let ok: bool = unsafe {
        msg_send![store, enumerateContactsWithFetchRequest: &*req, error: std::ptr::null_mut::<*mut NSError>(), usingBlock: &*block]
    };
    drop(block);
    if !ok {
        return Err("could not read contacts".into());
    }
    Ok(out.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn person(given: &str, family: &str, org: &str) -> Contact {
        Contact { given: given.into(), family: family.into(), org: org.into(), ..Default::default() }
    }

    #[test]
    fn names_fall_back_to_company_then_email() {
        assert_eq!(person("Ada", "Lovelace", "").name(), "Ada Lovelace");
        assert_eq!(person("", "Lovelace", "").name(), "Lovelace");
        assert_eq!(person("", "", "Analytical Engines").name(), "Analytical Engines");
        let mut c = person("", "", "");
        c.emails.push(Field { label: "work".into(), value: "ada@example.com".into() });
        assert_eq!(c.name(), "ada@example.com");
    }

    #[test]
    fn name_prefix_beats_word_prefix() {
        let mut list = vec![person("Mary", "Annsley", ""), person("Zed", "", ""), person("Anna", "Bell", "")];
        sort(&mut list, "ann");
        let names: Vec<String> = list.iter().map(Contact::name).collect();
        assert_eq!(names, ["Anna Bell", "Mary Annsley", "Zed"]);
    }

    #[test]
    fn status_never_prompts() {
        let s = status();
        assert!(["notDetermined", "restricted", "denied", "authorized", "limited"].contains(&s), "{s}");
    }
}
