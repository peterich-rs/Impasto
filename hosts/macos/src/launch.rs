//! Launch URL from AppKit open-URLs, a file path, or argv.

use objc2_foundation::{NSArray, NSString, NSURL};

/// Absolute URL string. Empty is not a launch URL.
pub(crate) fn from_absolute(url: &str) -> Option<String> {
    let url = url.trim();
    if url.is_empty() {
        None
    } else {
        Some(url.to_owned())
    }
}

pub(crate) fn from_nsurl(url: &NSURL) -> Option<String> {
    from_absolute(&url.absoluteString()?.to_string())
}

/// First non-empty URL in an `application:openURLs:` batch.
pub(crate) fn from_open_urls(urls: &NSArray<NSURL>) -> Option<String> {
    for index in 0..urls.count() {
        if let Some(url) = from_nsurl(&urls.objectAtIndex(index)) {
            return Some(url);
        }
    }
    None
}

/// `file://` URL for a path from `application:openFile:`.
pub(crate) fn from_file_path(path: &str) -> Option<String> {
    let path = path.trim();
    if path.is_empty() {
        return None;
    }
    from_nsurl(&NSURL::fileURLWithPath(&NSString::from_str(path)))
}

/// `--url <string>` or a `scheme://` token. `--js` consumes its value so a
/// script path that looks like a URL is not the launch URL.
pub(crate) fn from_args<I, S>(args: I) -> Option<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut found = None;
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        let arg = arg.as_ref();
        if arg == "--js" {
            let _ = args.next();
            continue;
        }
        if arg == "--dump-list" {
            continue;
        }
        if arg == "--url" {
            found = args.next().and_then(|next| from_absolute(next.as_ref()));
            continue;
        }
        if looks_like_url(arg) {
            found = from_absolute(arg);
        }
    }
    found
}

pub(crate) fn looks_like_url(arg: &str) -> bool {
    let Some((scheme, rest)) = arg.split_once("://") else {
        return false;
    };
    !scheme.is_empty() && scheme.bytes().all(|byte| byte.is_ascii_alphabetic()) && !rest.is_empty()
}

#[cfg(test)]
mod tests {
    use super::{from_absolute, from_args, from_file_path, looks_like_url};

    #[test]
    fn empty_absolute_is_not_a_url() {
        assert_eq!(from_absolute(""), None);
        assert_eq!(from_absolute("  "), None);
        assert_eq!(
            from_absolute("https://app.example/item"),
            Some("https://app.example/item".into())
        );
    }

    #[test]
    fn scheme_slash_slash_is_a_url() {
        assert!(looks_like_url("https://app.example/item"));
        assert!(looks_like_url("file:///tmp/a"));
        assert!(looks_like_url("myapp://open"));
        assert!(!looks_like_url("https://"));
        assert!(!looks_like_url("/tmp/a"));
        assert!(!looks_like_url("--js"));
        assert!(!looks_like_url("hosts/macos/assets"));
    }

    #[test]
    fn args_take_url_flag_or_scheme() {
        assert_eq!(
            from_args(["--js", "apps/gallery-solid/dist/counter.js"]),
            None
        );
        assert_eq!(from_args(["--js", "https://example.com/app.js"]), None);
        assert_eq!(
            from_args(["--url", "https://app.example/item"]),
            Some("https://app.example/item".into())
        );
        assert_eq!(
            from_args(["https://app.example/item", "--dump-list"]),
            Some("https://app.example/item".into())
        );
        assert_eq!(from_args(["--url"]), None);
        assert_eq!(from_args(["--url", ""]), None);
    }

    #[test]
    fn file_path_becomes_file_url() {
        objc2::rc::autoreleasepool(|_| {
            assert_eq!(from_file_path(""), None);
            assert_eq!(from_file_path("   "), None);
            let url = match from_file_path("/tmp/launch test.txt") {
                Some(url) => url,
                None => panic!("file url"),
            };
            assert!(url.starts_with("file://"), "{url}");
            assert!(url.contains("launch%20test.txt"), "{url}");
        });
    }
}
