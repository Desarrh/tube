//! Intégration au panneau multimédia de Windows (SystemMediaTransportControls) : titre, chaîne et
//! miniature dans le menu du volume et sur l'écran de verrouillage, et surtout les touches
//! multimédia du clavier (lecture/pause, suivante, précédente) qui pilotent Tube depuis n'importe où.
use windows::Foundation::{TypedEventHandler, Uri};
use windows::Media::*;
use windows::Storage::Streams::RandomAccessStreamReference;
use windows::Win32::Foundation::HWND;
use windows::Win32::System::WinRT::ISystemMediaTransportControlsInterop;
use windows::core::*;

#[derive(Clone, Copy, Debug)]
pub enum Btn {
    PlayPause,
    Next,
    Previous,
}

pub struct Smtc {
    c: SystemMediaTransportControls,
    shown: String,
    status: i32,
}

impl Smtc {
    pub fn new(hwnd: isize, send: impl Fn(Btn) + Send + Sync + 'static) -> Option<Smtc> {
        let interop = factory::<SystemMediaTransportControls, ISystemMediaTransportControlsInterop>().ok()?;
        let c: SystemMediaTransportControls = unsafe { interop.GetForWindow(HWND(hwnd as *mut _)) }.ok()?;
        c.SetIsPlayEnabled(true).ok()?;
        c.SetIsPauseEnabled(true).ok()?;
        c.SetIsNextEnabled(true).ok()?;
        c.SetIsPreviousEnabled(true).ok()?;
        c.ButtonPressed(&TypedEventHandler::new(move |_: Ref<SystemMediaTransportControls>, a: Ref<SystemMediaTransportControlsButtonPressedEventArgs>| {
            if let Some(a) = a.as_ref() {
                let b = a.Button()?;
                if b == SystemMediaTransportControlsButton::Play || b == SystemMediaTransportControlsButton::Pause {
                    send(Btn::PlayPause);
                } else if b == SystemMediaTransportControlsButton::Next {
                    send(Btn::Next);
                } else if b == SystemMediaTransportControlsButton::Previous {
                    send(Btn::Previous);
                }
            }
            Ok(())
        }))
        .ok()?;
        Some(Smtc { c, shown: String::new(), status: -1 })
    }

    /// `None` : rien en lecture (panneau masqué).
    pub fn update(&mut self, now: Option<(&str, &str, &str, &str, bool)>) {
        let Some((id, title, channel, thumb, playing)) = now else {
            if self.status != 0 {
                let _ = self.c.SetIsEnabled(false);
                self.status = 0;
                self.shown.clear();
            }
            return;
        };
        if self.shown != id {
            self.shown = id.to_string();
            let _ = (|| -> Result<()> {
                self.c.SetIsEnabled(true)?;
                let d = self.c.DisplayUpdater()?;
                d.SetType(MediaPlaybackType::Video)?;
                let v = d.VideoProperties()?;
                v.SetTitle(&HSTRING::from(title))?;
                v.SetSubtitle(&HSTRING::from(channel))?;
                if !thumb.is_empty() {
                    d.SetThumbnail(&RandomAccessStreamReference::CreateFromUri(&Uri::CreateUri(&HSTRING::from(thumb))?)?)?;
                }
                d.Update()
            })();
        }
        let st = if playing { 1 } else { 2 };
        if st != self.status {
            self.status = st;
            let _ = self.c.SetIsEnabled(true);
            let _ = self.c.SetPlaybackStatus(if playing { MediaPlaybackStatus::Playing } else { MediaPlaybackStatus::Paused });
        }
    }
}
