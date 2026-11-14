//! Desktop notifications on track change.

use neo_ncmp_domain::Song;

pub fn track_changed(song: &Song) {
    let summary = format!("{} — {}", song.display_artist(), song.display_title());
    let body = song.display_album().to_string();
    std::thread::spawn(move || {
        let _ = notify_rust::Notification::new()
            .appname("neo-ncmp")
            .summary(&summary)
            .body(&body)
            .timeout(notify_rust::Timeout::Milliseconds(4000))
            .show();
    });
}
