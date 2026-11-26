//! The pasteboard half of an insert: keep what the pasteboard holds, put the dictated text there
//! for the paste, then put back what was kept.
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

use crate::protocol::{
    should_restore, snapshot_restorable, ACCESS_BEHAVIOR_ALWAYS_DENY, CONCEALED_TYPE,
    TRANSIENT_TYPE,
};

/// One kept item: each of its types with the data it held.
type SavedItem = Vec<(Retained<NSString>, Retained<NSData>)>;

/// What the pasteboard held before the text was written.
pub struct Snapshot {
    items: Vec<SavedItem>,
    /// Types whose data could not be read (a provider that gave nothing). They are not restored.
    #[cfg_attr(not(test), allow(dead_code))]
    pub skipped: usize,
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

/// Keep every item and type of `pasteboard`. A type whose data cannot be read is left out, with
/// one fixed log line per process: the insert never waits on how faithfully the pasteboard can
/// be put back.
pub fn snapshot(pasteboard: &NSPasteboard) -> Snapshot {
    let mut items = Vec::new();
    let mut skipped = 0;
    if reads_denied(pasteboard) {
        return Snapshot {
            items,
            skipped,
            restorable: false,
        };
    }
    let mut reported = 0;
    if let Some(list) = pasteboard.pasteboardItems() {
        reported = list.len();
        for item in list.iter() {
            let mut saved = SavedItem::new();
            for kind in item.types().iter() {
                match item.dataForType(&kind) {
                    Some(data) => saved.push((kind, data)),
                    None => skipped += 1,
                }
            }
            if !saved.is_empty() {
                items.push(saved);
            }
        }
    }
    if skipped > 0 && !SKIP_REPORTED.swap(true, Ordering::Relaxed) {
        eprintln!("biorouter-textinsert: a pasteboard type could not be read and is not restored");
    }
    let restorable = snapshot_restorable(false, reported, items.len());
    Snapshot {
        items,
        skipped,
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

/// What [`restore`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RestoreOutcome {
    /// The pasteboard holds what it held before the text was written.
    Restored,
    /// Something was copied after the text; it is left alone.
    Moved,
    /// The pasteboard would not take the kept items back.
    Failed,
    /// What was there could not be read, so the text is left where a copy would leave it.
    NotRead,
}

/// Put `snapshot` back on `pasteboard`, only if the pasteboard still holds what the helper wrote
/// (its change count is still `written.count`). An empty snapshot of a readable pasteboard clears
/// the text.
pub fn restore(pasteboard: &NSPasteboard, snapshot: Snapshot, written: Written) -> RestoreOutcome {
    if !should_restore(written.count, pasteboard.changeCount()) {
        return RestoreOutcome::Moved;
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

#[cfg(test)]
mod tests {
    use super::*;
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
            assert_eq!((kept.len(), kept.skipped, kept.restorable), (3, 0, true));

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
