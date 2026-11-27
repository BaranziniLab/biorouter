//! The pasteboard half of an insert: keep what the pasteboard holds, put the dictated text there
//! for the paste, then put back what was kept, and say how the clipboard was left
//! ([`Restored`]).
//!
//! A password is not put back. When what the pasteboard held is marked concealed
//! (`org.nspasteboard.ConcealedType`, what password managers write), the snapshot keeps none of
//! its data and the pasteboard is cleared after the paste: a copy put back gets a new change
//! count, and a password manager that clears its copy by change count would then leave the
//! password on the clipboard for good.
//!
//! Every function takes the pasteboard it works on, so the tests run on private named pasteboards
//! (`NSPasteboard::pasteboardWithUniqueName`) and nothing here can reach the general pasteboard.
//! Which pasteboard the helper writes is decided once, in `mac.rs`.

use std::sync::atomic::{AtomicBool, Ordering};

use objc2::rc::Retained;
use objc2::runtime::{NSObjectProtocol, ProtocolObject};
use objc2::sel;
use objc2_app_kit::{
    NSPasteboard, NSPasteboardContentsOptions, NSPasteboardItem, NSPasteboardTypeString,
    NSPasteboardWriting,
};
use objc2_foundation::{NSArray, NSData, NSString};

pub use crate::protocol::RestoreOutcome;
use crate::protocol::{
    restored, should_restore, snapshot_restorable, Restored, ACCESS_BEHAVIOR_ALWAYS_DENY,
    CONCEALED_TYPE, MAX_SNAPSHOT_BYTES, TRANSIENT_TYPE,
};

/// One kept item: each of its types with the data it held.
type SavedItem = Vec<(Retained<NSString>, Retained<NSData>)>;

/// What the pasteboard held before the text was written.
pub struct Snapshot {
    items: Vec<SavedItem>,
    /// Types whose data could not be read (a provider that gave nothing), or that would have
    /// taken the snapshot past [`MAX_SNAPSHOT_BYTES`]. They are not restored.
    pub skipped: usize,
    /// What was there is marked concealed (a password): none of it is kept, and the pasteboard
    /// is cleared after the paste rather than refilled.
    pub concealed: bool,
    /// Whether putting this snapshot back would restore what was there. Not when the pasteboard
    /// could not be read at all: clearing it then would lose the person's copy and the text.
    pub restorable: bool,
}

impl Snapshot {
    /// How many items were kept.
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.items.len()
    }
}

static SKIP_REPORTED: AtomicBool = AtomicBool::new(false);

/// Reads one type of one item: `dataForType:`, or a test's stand-in.
pub type ReadType<'a> = &'a dyn Fn(&NSPasteboardItem, &NSString) -> Option<Retained<NSData>>;

/// Keep every item and type of `pasteboard`, up to [`MAX_SNAPSHOT_BYTES`]. A type whose data
/// cannot be read is left out, with one fixed log line per process: the insert never waits on
/// how faithfully the pasteboard can be put back.
pub fn snapshot(pasteboard: &NSPasteboard) -> Snapshot {
    snapshot_with(pasteboard, MAX_SNAPSHOT_BYTES, &|item, kind| {
        item.dataForType(kind)
    })
}

/// [`snapshot`], keeping at most `cap` bytes and reading each type with `read`. A type that
/// would take the snapshot past `cap` is left out, as an unreadable one is, and both count in
/// [`Snapshot::skipped`]. An item marked concealed is never read: the snapshot keeps nothing
/// and says so.
pub fn snapshot_with(pasteboard: &NSPasteboard, cap: usize, read: ReadType<'_>) -> Snapshot {
    let mut items = Vec::new();
    let mut skipped = 0;
    if reads_denied(pasteboard) {
        return Snapshot {
            items,
            skipped,
            concealed: false,
            restorable: false,
        };
    }
    let Some(list) = pasteboard.pasteboardItems() else {
        return Snapshot {
            items,
            skipped,
            concealed: false,
            restorable: snapshot_restorable(false, 0, 0),
        };
    };
    let concealed_type = NSString::from_str(CONCEALED_TYPE);
    let concealed = list
        .iter()
        .any(|item| item.types().iter().any(|kind| *kind == *concealed_type));
    if concealed {
        // A password: nothing of it is read, and the pasteboard is cleared after the paste.
        return Snapshot {
            items,
            skipped,
            concealed: true,
            restorable: true,
        };
    }
    let mut kept_bytes: usize = 0;
    for item in list.iter() {
        let mut saved = SavedItem::new();
        for kind in item.types().iter() {
            match read(&item, &kind) {
                Some(data) if kept_bytes.saturating_add(data.len()) <= cap => {
                    kept_bytes += data.len();
                    saved.push((kind, data));
                }
                Some(_) | None => skipped += 1,
            }
        }
        if !saved.is_empty() {
            items.push(saved);
        }
    }
    if skipped > 0 && !SKIP_REPORTED.swap(true, Ordering::Relaxed) {
        eprintln!("biorouter-textinsert: a pasteboard type could not be kept and is not restored");
    }
    let restorable = snapshot_restorable(false, list.len(), items.len());
    Snapshot {
        items,
        skipped,
        concealed: false,
        restorable,
    }
}

/// Whether the person set this app's pasteboard access to "deny" (macOS 15.4 and later, System
/// Settings): every read then comes back empty. Earlier systems have no such setting and never
/// deny.
fn reads_denied(pasteboard: &NSPasteboard) -> bool {
    pasteboard.respondsToSelector(sel!(accessBehavior))
        && pasteboard.accessBehavior().0 == ACCESS_BEHAVIOR_ALWAYS_DENY
}

/// What writing the text left on the pasteboard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Written {
    /// The pasteboard's change count once the text is on it: the restore goes ahead only while
    /// the count is still this one.
    pub count: isize,
    /// Whether the text is there. When it is not, the pasteboard may already have been cleared,
    /// so the snapshot is still put back.
    pub ok: bool,
}

fn item_with_text(text: &str) -> Option<Retained<NSPasteboardItem>> {
    let item = NSPasteboardItem::new();
    let empty = NSData::new();
    // SAFETY: a constant AppKit defines, valid for the life of the process.
    let string_type = unsafe { NSPasteboardTypeString };
    let ok = item.setString_forType(&NSString::from_str(text), string_type)
        && item.setData_forType(&empty, &NSString::from_str(TRANSIENT_TYPE))
        && item.setData_forType(&empty, &NSString::from_str(CONCEALED_TYPE));
    ok.then_some(item)
}

/// Put `text` on `pasteboard` as its only item, marked transient and concealed so clipboard
/// managers leave it out, and kept to this Mac so Universal Clipboard does not send it on.
pub fn write_transient_text(pasteboard: &NSPasteboard, text: &str) -> Written {
    // The count after the clear is the one the helper owns: `writeObjects` does not change it,
    // which `the_count_kept_is_the_count_after_the_write` measures, so reading it here leaves no
    // gap in which another app's copy could be taken for the helper's own.
    let count =
        pasteboard.prepareForNewContentsWithOptions(NSPasteboardContentsOptions::CurrentHostOnly);
    let ok = item_with_text(text).is_some_and(|item| {
        let items = NSArray::from_retained_slice(&[ProtocolObject::from_retained(item)]);
        pasteboard.writeObjects(&items)
    });
    Written { count, ok }
}

/// Put `snapshot` back on `pasteboard`, only if the pasteboard still holds what the helper wrote
/// (its change count is still `written.count`). An empty snapshot of a readable pasteboard clears
/// the text, and so does a concealed one, which is never refilled.
pub fn restore(pasteboard: &NSPasteboard, snapshot: Snapshot, written: Written) -> RestoreOutcome {
    if !should_restore(written.count, pasteboard.changeCount()) {
        return RestoreOutcome::Moved;
    }
    if snapshot.concealed {
        pasteboard.clearContents();
        return RestoreOutcome::Cleared;
    }
    if !snapshot.restorable {
        return RestoreOutcome::NotRead;
    }
    pasteboard.clearContents();
    if snapshot.items.is_empty() {
        return RestoreOutcome::Restored;
    }
    let items: Vec<Retained<ProtocolObject<dyn NSPasteboardWriting>>> = snapshot
        .items
        .iter()
        .map(|saved| {
            let item = NSPasteboardItem::new();
            for (kind, data) in saved {
                // A type the pasteboard refuses is skipped; the rest still go back.
                item.setData_forType(data, kind);
            }
            ProtocolObject::from_retained(item)
        })
        .collect();
    if pasteboard.writeObjects(&NSArray::from_retained_slice(&items)) {
        RestoreOutcome::Restored
    } else {
        RestoreOutcome::Failed
    }
}

/// [`restore`], and how that left the person's clipboard: what the `inserted` answer says.
pub fn put_back(pasteboard: &NSPasteboard, snapshot: Snapshot, written: Written) -> Restored {
    let incomplete = snapshot.skipped > 0;
    restored(restore(pasteboard, snapshot, written), incomplete)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::Restored;
    use objc2::msg_send;
    use objc2::rc::autoreleasepool;
    use objc2_app_kit::{NSPasteboardTypeFileURL, NSPasteboardTypePNG, NSPasteboardTypeRTF};

    /// A private pasteboard of its own for each test, released when the test ends. No test
    /// touches the general pasteboard.
    struct TestPasteboard(Retained<NSPasteboard>);

    impl TestPasteboard {
        fn new() -> Self {
            Self(NSPasteboard::pasteboardWithUniqueName())
        }
    }

    impl Drop for TestPasteboard {
        fn drop(&mut self) {
            // SAFETY: `releaseGlobally` takes no arguments and returns nothing; the pasteboard is
            // not used after it.
            let () = unsafe { msg_send![&*self.0, releaseGlobally] };
        }
    }

    impl std::ops::Deref for TestPasteboard {
        type Target = NSPasteboard;
        fn deref(&self) -> &NSPasteboard {
            &self.0
        }
    }

    /// Every item, every type and its bytes, as plain values to compare.
    fn contents(pasteboard: &NSPasteboard) -> Vec<Vec<(String, Vec<u8>)>> {
        pasteboard
            .pasteboardItems()
            .map(|items| {
                items
                    .iter()
                    .map(|item| {
                        item.types()
                            .iter()
                            .map(|kind| {
                                let data = item.dataForType(&kind).map(|data| data.to_vec());
                                (kind.to_string(), data.unwrap_or_default())
                            })
                            .collect()
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    fn put(pasteboard: &NSPasteboard, items: &[&[(&NSString, &[u8])]]) {
        pasteboard.clearContents();
        let items: Vec<Retained<ProtocolObject<dyn NSPasteboardWriting>>> = items
            .iter()
            .map(|types| {
                let item = NSPasteboardItem::new();
                for (kind, bytes) in *types {
                    assert!(item.setData_forType(&NSData::with_bytes(bytes), kind));
                }
                ProtocolObject::from_retained(item)
            })
            .collect();
        assert!(pasteboard.writeObjects(&NSArray::from_retained_slice(&items)));
    }

    fn the_text(pasteboard: &NSPasteboard) -> Option<String> {
        // SAFETY: an AppKit constant.
        pasteboard
            .stringForType(unsafe { NSPasteboardTypeString })
            .map(|text| text.to_string())
    }

    /// A one-pixel PNG.
    const PNG: &[u8] = &[
        0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44,
        0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1f,
        0x15, 0xc4, 0x89, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9c, 0x63, 0xf8,
        0xcf, 0xc0, 0xf0, 0x1f, 0x00, 0x05, 0x00, 0x01, 0xff, 0x89, 0x99, 0x3d, 0x1d, 0x00, 0x00,
        0x00, 0x00, 0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
    ];

    #[test]
    fn text_rtf_an_image_and_a_file_url_are_put_back_as_they_were() {
        autoreleasepool(|_| {
            let pasteboard = TestPasteboard::new();
            // SAFETY: AppKit constants.
            let (string, rtf, png, file_url) = unsafe {
                (
                    NSPasteboardTypeString,
                    NSPasteboardTypeRTF,
                    NSPasteboardTypePNG,
                    NSPasteboardTypeFileURL,
                )
            };
            put(
                &pasteboard,
                &[
                    &[
                        (string, b"what the person copied".as_slice()),
                        (rtf, br"{\rtf1\ansi what the {\b person} copied}".as_slice()),
                    ],
                    &[(png, PNG)],
                    &[(
                        file_url,
                        b"file:///Users/someone/Documents/notes.txt".as_slice(),
                    )],
                ],
            );
            let before = contents(&pasteboard);
            assert_eq!(before.len(), 3);

            let kept = snapshot(&pasteboard);
            assert_eq!(
                (kept.len(), kept.skipped, kept.concealed, kept.restorable),
                (3, 0, false, true)
            );

            let written = write_transient_text(&pasteboard, "the dictated words");
            assert!(written.ok);
            assert_eq!(the_text(&pasteboard).as_deref(), Some("the dictated words"));

            assert_eq!(
                restore(&pasteboard, kept, written),
                RestoreOutcome::Restored
            );
            assert_eq!(contents(&pasteboard), before);
        });
    }

    #[test]
    fn the_text_is_the_only_item_and_carries_both_markers() {
        autoreleasepool(|_| {
            let pasteboard = TestPasteboard::new();
            put(
                &pasteboard,
                &[&[(unsafe { NSPasteboardTypeString }, b"old".as_slice())]],
            );
            let written = write_transient_text(&pasteboard, "new words");
            assert!(written.ok);
            let now = contents(&pasteboard);
            assert_eq!(now.len(), 1);
            let types: Vec<&str> = now[0].iter().map(|(kind, _)| kind.as_str()).collect();
            assert!(types.contains(&TRANSIENT_TYPE));
            assert!(types.contains(&CONCEALED_TYPE));
            assert!(types.contains(&"public.utf8-plain-text"));
            assert_eq!(the_text(&pasteboard).as_deref(), Some("new words"));
        });
    }

    #[test]
    fn a_copy_made_after_the_text_is_never_overwritten() {
        autoreleasepool(|_| {
            let pasteboard = TestPasteboard::new();
            // SAFETY: an AppKit constant.
            let string = unsafe { NSPasteboardTypeString };
            put(&pasteboard, &[&[(string, b"before".as_slice())]]);
            let kept = snapshot(&pasteboard);
            let written = write_transient_text(&pasteboard, "dictated");
            assert!(written.ok);

            // The person copies something while the paste is under way.
            pasteboard.clearContents();
            assert!(pasteboard.setString_forType(&NSString::from_str("the person's copy"), string));

            assert_eq!(restore(&pasteboard, kept, written), RestoreOutcome::Moved);
            assert_eq!(the_text(&pasteboard).as_deref(), Some("the person's copy"));
        });
    }

    #[test]
    fn an_empty_pasteboard_is_left_empty() {
        autoreleasepool(|_| {
            let pasteboard = TestPasteboard::new();
            pasteboard.clearContents();
            let kept = snapshot(&pasteboard);
            assert_eq!(kept.len(), 0);
            // An empty pasteboard that could be read is put back as empty.
            assert!(kept.restorable);
            assert!(!reads_denied(&pasteboard));
            let written = write_transient_text(&pasteboard, "dictated");
            assert!(written.ok);
            assert_eq!(
                restore(&pasteboard, kept, written),
                RestoreOutcome::Restored
            );
            assert_eq!(the_text(&pasteboard), None);
            assert!(contents(&pasteboard).is_empty());
        });
    }

    /// F5 (W-A1): a password a password manager copied is not put back. The snapshot keeps none
    /// of it, and after the paste the pasteboard is cleared, so the text goes too and the
    /// password manager's own clearing, which goes by change count, is not defeated.
    #[test]
    fn a_concealed_original_is_cleared_not_restored() {
        autoreleasepool(|_| {
            let pasteboard = TestPasteboard::new();
            // SAFETY: an AppKit constant.
            let string = unsafe { NSPasteboardTypeString };
            let concealed = NSString::from_str(CONCEALED_TYPE);
            put(
                &pasteboard,
                &[&[
                    (string, b"correct horse battery staple".as_slice()),
                    (&concealed, b"".as_slice()),
                ]],
            );
            let kept = snapshot(&pasteboard);
            assert!(kept.concealed);
            assert_eq!(kept.len(), 0, "nothing of a password is kept");
            let written = write_transient_text(&pasteboard, "the dictated words");
            assert!(written.ok);
            assert_eq!(put_back(&pasteboard, kept, written), Restored::Cleared);
            assert!(contents(&pasteboard).is_empty());
            assert_eq!(the_text(&pasteboard), None);
            assert!(pasteboard.changeCount() > written.count);

            // A copy made after the text is still left alone.
            put(
                &pasteboard,
                &[&[
                    (string, b"another secret".as_slice()),
                    (&concealed, b"".as_slice()),
                ]],
            );
            let kept = snapshot(&pasteboard);
            let written = write_transient_text(&pasteboard, "dictated");
            pasteboard.clearContents();
            assert!(pasteboard.setString_forType(&NSString::from_str("copied later"), string));
            assert_eq!(put_back(&pasteboard, kept, written), Restored::Moved);
            assert_eq!(the_text(&pasteboard).as_deref(), Some("copied later"));
        });
    }

    /// Types of this test's own, which AppKit derives nothing from, so what a test puts there
    /// is exactly what the pasteboard reports.
    fn own_type(name: &str) -> Retained<NSString> {
        NSString::from_str(&format!("org.biorouter.textinsert-test.{name}"))
    }

    /// F4 (W-A2): a type that cannot be read is left out, the rest goes back, and the answer
    /// says `partial`. The read is a stand-in that refuses one type, as a provider that gives
    /// nothing would.
    #[test]
    fn an_unreadable_type_makes_the_restore_partial_and_the_rest_comes_back() {
        autoreleasepool(|_| {
            let pasteboard = TestPasteboard::new();
            let (text, rich, image) = (own_type("text"), own_type("rich"), own_type("image"));
            put(
                &pasteboard,
                &[
                    &[
                        (&text, b"what the person copied".as_slice()),
                        (&rich, br"{\rtf1\ansi copied}".as_slice()),
                    ],
                    &[(&image, PNG)],
                ],
            );
            let before = contents(&pasteboard);
            let kept = snapshot_with(&pasteboard, MAX_SNAPSHOT_BYTES, &|item, kind| {
                if *kind == *rich {
                    None
                } else {
                    item.dataForType(kind)
                }
            });
            assert_eq!((kept.len(), kept.skipped, kept.restorable), (2, 1, true));
            let written = write_transient_text(&pasteboard, "the dictated words");
            assert_eq!(put_back(&pasteboard, kept, written), Restored::Partial);
            let after = contents(&pasteboard);
            assert_eq!(
                after,
                vec![vec![before[0][0].clone()], before[1].clone()],
                "the text and the image came back, the unreadable type did not"
            );
        });
    }

    /// F4 (W-A2): the snapshot keeps at most its cap; a type that would take it past is left
    /// out and reported, and what fits still comes back.
    #[test]
    fn types_past_the_snapshot_cap_are_skipped_and_reported() {
        autoreleasepool(|_| {
            let pasteboard = TestPasteboard::new();
            let (small, large, image) = (own_type("small"), own_type("large"), own_type("image"));
            let big = vec![b'x'; 4_096];
            put(
                &pasteboard,
                &[
                    &[(&small, b"small".as_slice())],
                    &[(&large, big.as_slice())],
                    &[(&image, PNG)],
                ],
            );
            let before = contents(&pasteboard);
            let cap = b"small".len() + PNG.len();
            let kept = snapshot_with(&pasteboard, cap, &|item, kind| item.dataForType(kind));
            assert_eq!((kept.len(), kept.skipped, kept.restorable), (2, 1, true));
            let written = write_transient_text(&pasteboard, "dictated");
            assert_eq!(put_back(&pasteboard, kept, written), Restored::Partial);
            assert_eq!(
                contents(&pasteboard),
                vec![before[0].clone(), before[2].clone()]
            );

            // Within the cap nothing is left out, and everything is `all`.
            put(
                &pasteboard,
                &[&[(&small, b"small".as_slice())], &[(&image, PNG)]],
            );
            let before = contents(&pasteboard);
            let kept = snapshot_with(&pasteboard, cap, &|item, kind| item.dataForType(kind));
            assert_eq!(kept.skipped, 0);
            let written = write_transient_text(&pasteboard, "dictated");
            assert_eq!(put_back(&pasteboard, kept, written), Restored::All);
            assert_eq!(contents(&pasteboard), before);
            assert_eq!(MAX_SNAPSHOT_BYTES, 64 * 1024 * 1024);
        });
    }

    /// What `put_back` answers for the other ways a restore ends: a copy made after the text,
    /// a pasteboard that could not be read (the text stays), and an empty one.
    #[test]
    fn put_back_says_how_the_clipboard_was_left() {
        autoreleasepool(|_| {
            let pasteboard = TestPasteboard::new();
            // SAFETY: an AppKit constant.
            let string = unsafe { NSPasteboardTypeString };
            put(&pasteboard, &[&[(string, b"before".as_slice())]]);
            let kept = snapshot(&pasteboard);
            let written = write_transient_text(&pasteboard, "dictated");
            assert_eq!(put_back(&pasteboard, kept, written), Restored::All);
            assert_eq!(the_text(&pasteboard).as_deref(), Some("before"));

            // Every type unreadable: nothing is put back and the text stays.
            put(&pasteboard, &[&[(string, b"before".as_slice())]]);
            let kept = snapshot_with(&pasteboard, MAX_SNAPSHOT_BYTES, &|_, _| None);
            assert!(!kept.restorable);
            let written = write_transient_text(&pasteboard, "dictated");
            assert_eq!(put_back(&pasteboard, kept, written), Restored::KeptText);
            assert_eq!(the_text(&pasteboard).as_deref(), Some("dictated"));

            pasteboard.clearContents();
            let kept = snapshot(&pasteboard);
            let written = write_transient_text(&pasteboard, "dictated");
            assert_eq!(put_back(&pasteboard, kept, written), Restored::All);
            assert!(contents(&pasteboard).is_empty());
        });
    }

    #[test]
    fn the_count_kept_is_the_count_after_the_write() {
        autoreleasepool(|_| {
            let pasteboard = TestPasteboard::new();
            let before = pasteboard.changeCount();
            let written = write_transient_text(&pasteboard, "dictated");
            assert!(written.ok);
            assert!(written.count > before);
            assert_eq!(pasteboard.changeCount(), written.count);
        });
    }
}
