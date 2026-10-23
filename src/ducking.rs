use std::sync::mpsc::{channel, Sender};
use std::sync::OnceLock;

enum Cmd {
    Duck,
    Restore,
}

static TX: OnceLock<Sender<Cmd>> = OnceLock::new();

pub fn init(level: f32) {
    let (tx, rx) = channel::<Cmd>();
    if TX.set(tx).is_err() {
        return;
    }
    std::thread::spawn(move || unsafe {
        use windows::Win32::Media::Audio::{
            eConsole, eRender, IAudioSessionControl2, IAudioSessionManager2, IMMDeviceEnumerator,
            ISimpleAudioVolume, MMDeviceEnumerator,
        };
        use windows::core::Interface;
        use windows::Win32::System::Com::{
            CoCreateInstance, CoInitializeEx, CLSCTX_ALL, COINIT_APARTMENTTHREADED,
        };

        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let mut saved: Vec<(ISimpleAudioVolume, f32)> = Vec::new();
        let our_pid = std::process::id();

        while let Ok(cmd) = rx.recv() {
            match cmd {
                Cmd::Duck => {
                    if !saved.is_empty() {
                        continue;
                    }
                    let Ok(enumerator) =
                        CoCreateInstance::<_, IMMDeviceEnumerator>(&MMDeviceEnumerator, None, CLSCTX_ALL)
                    else {
                        continue;
                    };
                    let Ok(device) = enumerator.GetDefaultAudioEndpoint(eRender, eConsole) else {
                        continue;
                    };
                    let Ok(manager) =
                        device.Activate::<IAudioSessionManager2>(CLSCTX_ALL, None)
                    else {
                        continue;
                    };
                    let Ok(sessions) = manager.GetSessionEnumerator() else {
                        continue;
                    };
                    let count = sessions.GetCount().unwrap_or(0);
                    for i in 0..count {
                        let Ok(ctrl) = sessions.GetSession(i) else {
                            continue;
                        };
                        if let Ok(ctrl2) = ctrl.cast::<IAudioSessionControl2>() {
                            if let Ok(pid) = ctrl2.GetProcessId() {
                                if pid == our_pid || pid == 0 {
                                    continue;
                                }
                            }
                        }
                        if let Ok(vol) = ctrl.cast::<ISimpleAudioVolume>() {
                            if let Ok(current) = vol.GetMasterVolume() {
                                if vol.SetMasterVolume((current * level).clamp(0.0, 1.0), std::ptr::null()).is_ok() {
                                    saved.push((vol, current));
                                }
                            }
                        }
                    }
                }
                Cmd::Restore => {
                    for (vol, previous) in saved.drain(..) {
                        let _ = vol.SetMasterVolume(previous, std::ptr::null());
                    }
                }
            }
        }
    });
}

pub fn duck() {
    if let Some(tx) = TX.get() {
        let _ = tx.send(Cmd::Duck);
    }
}

pub fn restore() {
    if let Some(tx) = TX.get() {
        let _ = tx.send(Cmd::Restore);
    }
}

