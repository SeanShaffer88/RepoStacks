//! Token protection via Windows DPAPI (CryptProtectData), called through
//! direct FFI so the window needs no extra crates. The protected blob is
//! base64-encoded into the settings file; the plain token never touches
//! disk and is only unprotected in memory, on demand.

#[cfg(windows)]
mod imp {
    use std::ffi::c_void;

    #[repr(C)]
    struct DataBlob {
        cb_data: u32,
        pb_data: *mut u8,
    }

    #[link(name = "crypt32")]
    extern "system" {
        fn CryptProtectData(
            data_in: *mut DataBlob,
            data_descr: *const u16,
            optional_entropy: *mut DataBlob,
            reserved: *mut c_void,
            prompt_struct: *mut c_void,
            flags: u32,
            data_out: *mut DataBlob,
        ) -> i32;
        fn CryptUnprotectData(
            data_in: *mut DataBlob,
            data_descr: *mut *mut u16,
            optional_entropy: *mut DataBlob,
            reserved: *mut c_void,
            prompt_struct: *mut c_void,
            flags: u32,
            data_out: *mut DataBlob,
        ) -> i32;
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn LocalFree(mem: *mut c_void) -> *mut c_void;
    }

    const CRYPTPROTECT_UI_FORBIDDEN: u32 = 0x1;

    pub fn protect(plain: &[u8]) -> Result<Vec<u8>, String> {
        let mut in_blob = DataBlob {
            cb_data: plain.len() as u32,
            pb_data: plain.as_ptr() as *mut u8,
        };
        let mut out_blob = DataBlob {
            cb_data: 0,
            pb_data: std::ptr::null_mut(),
        };
        let ok = unsafe {
            CryptProtectData(
                &mut in_blob,
                std::ptr::null(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut out_blob,
            )
        };
        if ok == 0 {
            return Err("DPAPI could not protect the token".into());
        }
        let out = unsafe {
            let slice = std::slice::from_raw_parts(out_blob.pb_data, out_blob.cb_data as usize);
            let v = slice.to_vec();
            LocalFree(out_blob.pb_data as *mut c_void);
            v
        };
        Ok(out)
    }

    pub fn unprotect(blob: &[u8]) -> Result<Vec<u8>, String> {
        let mut in_blob = DataBlob {
            cb_data: blob.len() as u32,
            pb_data: blob.as_ptr() as *mut u8,
        };
        let mut out_blob = DataBlob {
            cb_data: 0,
            pb_data: std::ptr::null_mut(),
        };
        let ok = unsafe {
            CryptUnprotectData(
                &mut in_blob,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut out_blob,
            )
        };
        if ok == 0 {
            return Err("DPAPI could not unprotect the token (different user or machine?)".into());
        }
        let out = unsafe {
            let slice = std::slice::from_raw_parts(out_blob.pb_data, out_blob.cb_data as usize);
            let v = slice.to_vec();
            LocalFree(out_blob.pb_data as *mut c_void);
            v
        };
        Ok(out)
    }
}

const B64: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

pub fn b64_encode(data: &[u8]) -> String {
    let mut out = String::new();
    for chunk in data.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
        out.push(B64[(n >> 18) as usize & 63] as char);
        out.push(B64[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            B64[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            B64[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

pub fn b64_decode(text: &str) -> Result<Vec<u8>, String> {
    let mut vals = Vec::new();
    for c in text.chars() {
        match c {
            '=' => break,
            c => {
                let v = B64
                    .iter()
                    .position(|&b| b as char == c)
                    .ok_or_else(|| "invalid base64".to_string())?;
                vals.push(v as u8);
            }
        }
    }
    let mut out = Vec::new();
    for chunk in vals.chunks(4) {
        if chunk.len() < 2 {
            break;
        }
        let n = chunk.iter().fold(0u32, |acc, &v| (acc << 6) | v as u32) << (6 * (4 - chunk.len()));
        out.push((n >> 16) as u8);
        if chunk.len() > 2 {
            out.push((n >> 8) as u8);
        }
        if chunk.len() > 3 {
            out.push(n as u8);
        }
    }
    Ok(out)
}

/// Protect a token for storage. Windows-only by design (DPAPI).
pub fn protect_token(token: &str) -> Result<String, String> {
    #[cfg(windows)]
    {
        let blob = imp::protect(token.as_bytes())?;
        Ok(b64_encode(&blob))
    }
    #[cfg(not(windows))]
    {
        let _ = token;
        Err("token storage uses Windows DPAPI and is only available on Windows".into())
    }
}

/// Recover a token in memory. Never logged, never written anywhere.
pub fn unprotect_token(stored: &str) -> Result<String, String> {
    #[cfg(windows)]
    {
        let blob = b64_decode(stored)?;
        let plain = imp::unprotect(&blob)?;
        String::from_utf8(plain).map_err(|_| "stored token is not valid text".into())
    }
    #[cfg(not(windows))]
    {
        let _ = stored;
        Err("token storage uses Windows DPAPI and is only available on Windows".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_round_trip() {
        for input in [
            &b""[..],
            b"f",
            b"fo",
            b"foo",
            b"foob",
            b"fooba",
            b"foobar",
            &[0u8, 255, 1, 254][..],
        ] {
            assert_eq!(b64_decode(&b64_encode(input)).unwrap(), input);
        }
        assert_eq!(b64_encode(b"foobar"), "Zm9vYmFy");
    }
}
