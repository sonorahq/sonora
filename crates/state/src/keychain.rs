//! The OS keychain, for the few secrets Sonora keeps outside its own files: the Secret Service
//! on Linux, the login keychain on macOS and the Credential Manager on Windows.
//!
//! Each store is reached exactly the way GPUI reached it when it provided credentials, with the
//! same labels, attributes and target names, so what an earlier build stored is still found.
//! Ported from GPUI's platform crates (Zed Industries, Apache-2.0).
//!
//! A platform with none of these reads nothing and refuses to write, which a caller takes as a
//! desktop with no keychain.

use anyhow::Result;

/// Reads the account name and secret stored for `url`, or `None` when nothing is stored.
pub(crate) async fn read(url: &str) -> Result<Option<(String, Vec<u8>)>> {
    platform::read(url).await
}

/// Stores `password` for `url` under the account name `user`, replacing what was there.
pub(crate) async fn write(url: &str, user: &str, password: &[u8]) -> Result<()> {
    platform::write(url, user, password).await
}

/// Removes what is stored for `url`.
#[allow(dead_code)]
pub(crate) async fn delete(url: &str) -> Result<()> {
    platform::delete(url).await
}

#[cfg(any(target_os = "linux", target_os = "freebsd"))]
mod platform {
    use anyhow::{Context as _, Result};

    /// The label every item is stored under. It is GPUI's, kept so that items stored by earlier
    /// builds are still found.
    const KEYRING_LABEL: &str = "zed-github-account";

    pub(super) async fn write(url: &str, username: &str, password: &[u8]) -> Result<()> {
        let keyring = oo7::Keyring::new().await?;
        keyring.unlock().await?;
        keyring
            .create_item(
                KEYRING_LABEL,
                &vec![("url", url), ("username", username)],
                password.to_vec(),
                true,
            )
            .await?;
        Ok(())
    }

    pub(super) async fn read(url: &str) -> Result<Option<(String, Vec<u8>)>> {
        let keyring = oo7::Keyring::new().await?;
        keyring.unlock().await?;

        let items = keyring.search_items(&vec![("url", url)]).await?;

        for item in items.into_iter() {
            if item.label().await.is_ok_and(|label| label == KEYRING_LABEL) {
                let attributes = item.attributes().await?;
                let username = attributes
                    .get("username")
                    .context("Cannot find username in stored credentials")?;
                item.unlock().await?;
                let secret = item.secret().await?;

                // we lose the zeroizing capabilities at this boundary
                return Ok(Some((username.to_string(), secret.to_vec())));
            }
        }
        Ok(None)
    }

    pub(super) async fn delete(url: &str) -> Result<()> {
        let keyring = oo7::Keyring::new().await?;
        keyring.unlock().await?;

        let items = keyring.search_items(&vec![("url", url)]).await?;

        for item in items.into_iter() {
            if item.label().await.is_ok_and(|label| label == KEYRING_LABEL) {
                item.delete().await?;
                return Ok(());
            }
        }

        Ok(())
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use std::ptr;

    use anyhow::{Result, anyhow, bail};
    use objc2_core_foundation::{
        CFData, CFDictionary, CFRetained, CFString, CFType, kCFBooleanTrue,
    };
    use objc2_security::{
        SecItemAdd, SecItemCopyMatching, SecItemDelete, SecItemUpdate, errSecItemNotFound,
        errSecSuccess, errSecUserCanceled, kSecAttrAccount, kSecAttrServer, kSecClass,
        kSecClassInternetPassword, kSecReturnAttributes, kSecReturnData, kSecValueData,
    };

    /// Which keychain call failed, for its error.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Operation {
        Read,
        Update,
        Create,
        Delete,
    }

    pub(super) async fn write(url: &str, username: &str, password: &[u8]) -> Result<()> {
        let (url, username, password) = (url.to_owned(), username.to_owned(), password.to_vec());
        tokio::task::spawn_blocking(move || write_now(&url, &username, &password)).await?
    }

    pub(super) async fn read(url: &str) -> Result<Option<(String, Vec<u8>)>> {
        let url = url.to_owned();
        tokio::task::spawn_blocking(move || read_now(&url)).await?
    }

    pub(super) async fn delete(url: &str) -> Result<()> {
        let url = url.to_owned();
        tokio::task::spawn_blocking(move || delete_now(&url)).await?
    }

    fn write_now(server: &str, account: &str, password: &[u8]) -> Result<()> {
        let server = CFString::from_str(server);
        let account = CFString::from_str(account);
        let password = CFData::from_bytes(password);
        // SAFETY: Reading immutable constants exported by the Security framework.
        let (class_key, server_key, account_key, data_key, internet_password) = unsafe {
            (
                kSecClass,
                kSecAttrServer,
                kSecAttrAccount,
                kSecValueData,
                kSecClassInternetPassword,
            )
        };
        let query = CFDictionary::<CFString, CFType>::from_slices(
            &[class_key, server_key],
            &[internet_password, &server],
        );
        let attributes = CFDictionary::<CFString, CFType>::from_slices(
            &[account_key, data_key],
            &[&account, &password],
        );
        let mut operation = Operation::Update;
        // SAFETY: Both dictionaries map Security attribute keys to CF values.
        let mut status = unsafe { SecItemUpdate(query.as_opaque(), attributes.as_opaque()) };
        if status == errSecItemNotFound {
            operation = Operation::Create;
            let attributes = CFDictionary::<CFString, CFType>::from_slices(
                &[class_key, server_key, account_key, data_key],
                &[internet_password, &server, &account, &password],
            );
            // SAFETY: The dictionary maps Security attribute keys to CF values, and a
            // null result pointer asks Security not to return the new item.
            status = unsafe { SecItemAdd(attributes.as_opaque(), ptr::null_mut()) };
        }
        check_status(operation, status)
    }

    fn read_now(server: &str) -> Result<Option<(String, Vec<u8>)>> {
        let server = CFString::from_str(server);
        // SAFETY: Reading immutable constants exported by Core Foundation and Security.
        let (
            true_value,
            class_key,
            server_key,
            return_attributes_key,
            return_data_key,
            internet_password,
        ) = unsafe {
            (
                kCFBooleanTrue,
                kSecClass,
                kSecAttrServer,
                kSecReturnAttributes,
                kSecReturnData,
                kSecClassInternetPassword,
            )
        };
        let true_value =
            true_value.ok_or_else(|| anyhow!("Core Foundation true value unavailable"))?;
        let query = CFDictionary::<CFString, CFType>::from_slices(
            &[
                class_key,
                server_key,
                return_attributes_key,
                return_data_key,
            ],
            &[internet_password, &server, true_value, true_value],
        );
        let mut result = ptr::null();
        // SAFETY: The query maps Security attribute keys to CF values, and `result`
        // is a valid out-pointer.
        let status = unsafe { SecItemCopyMatching(query.as_opaque(), &mut result) };
        if !read_status(status)? {
            return Ok(None);
        }
        let result = ptr::NonNull::new(result.cast_mut())
            .ok_or_else(|| anyhow!("invalid keychain response: keychain returned no item"))?;
        // SAFETY: SecItemCopyMatching follows the Create rule, so we own this reference.
        let result = unsafe { CFRetained::from_raw(result) }
            .downcast::<CFDictionary>()
            .map_err(|_| {
                anyhow!("invalid keychain response: keychain item was not a dictionary")
            })?;
        // SAFETY: Security returns CFType keys and values; each value is downcast below.
        let result = unsafe { result.cast_unchecked::<CFType, CFType>() };
        // SAFETY: Reading immutable constants exported by the Security framework.
        let (account_key, data_key) = unsafe { (kSecAttrAccount, kSecValueData) };
        let account = result
            .get(account_key)
            .ok_or_else(|| anyhow!("invalid keychain response: account was missing"))?
            .downcast::<CFString>()
            .map_err(|_| anyhow!("invalid keychain response: account was not a string"))?;
        let password = result
            .get(data_key)
            .ok_or_else(|| anyhow!("invalid keychain response: password was missing"))?
            .downcast::<CFData>()
            .map_err(|_| anyhow!("invalid keychain response: password was not data"))?;
        Ok(Some((account.to_string(), password.to_vec())))
    }

    fn delete_now(server: &str) -> Result<()> {
        let server = CFString::from_str(server);
        // SAFETY: Reading immutable constants exported by the Security framework.
        let (class_key, server_key, internet_password) =
            unsafe { (kSecClass, kSecAttrServer, kSecClassInternetPassword) };
        let query = CFDictionary::<CFString, CFType>::from_slices(
            &[class_key, server_key],
            &[internet_password, &server],
        );
        // SAFETY: The query maps Security attribute keys to CF values.
        let status = unsafe { SecItemDelete(query.as_opaque()) };
        check_status(Operation::Delete, status)
    }

    /// Whether a read found an item. Nothing stored and a prompt the user declined both read as
    /// nothing, which is what tells them apart from a keychain that failed.
    fn read_status(status: i32) -> Result<bool> {
        if status == errSecItemNotFound || status == errSecUserCanceled {
            Ok(false)
        } else {
            check_status(Operation::Read, status)?;
            Ok(true)
        }
    }

    fn check_status(operation: Operation, status: i32) -> Result<()> {
        if status != errSecSuccess {
            bail!("keychain {operation:?} failed with OSStatus {status}");
        }
        Ok(())
    }
}

#[cfg(target_os = "windows")]
mod platform {
    use anyhow::{Result, anyhow, bail};
    use windows::Win32::Foundation::ERROR_NOT_FOUND;
    use windows::Win32::Security::Credentials::{
        CRED_FLAGS, CRED_MAX_CREDENTIAL_BLOB_SIZE, CRED_PERSIST_LOCAL_MACHINE, CRED_TYPE_GENERIC,
        CREDENTIALW, CredDeleteW, CredFree, CredReadW, CredWriteW,
    };
    use windows::Win32::System::SystemInformation::GetSystemTimeAsFileTime;
    use windows::core::{PCWSTR, PWSTR};

    pub(super) async fn write(url: &str, username: &str, password: &[u8]) -> Result<()> {
        // CredWriteW rejects larger blobs with the opaque RPC error
        // 0x800706F7 "The stub received bad data", so fail with a clear
        // message instead.
        if password.len() > CRED_MAX_CREDENTIAL_BLOB_SIZE as usize {
            bail!(
                "credential for {url} is {} bytes, which exceeds the Windows Credential Manager \
                 limit of {CRED_MAX_CREDENTIAL_BLOB_SIZE} bytes",
                password.len()
            );
        }
        let password = password.to_vec();
        let mut username: Vec<u16> = username.encode_utf16().chain(Some(0)).collect();
        let mut target_name = target_name(url);
        tokio::task::spawn_blocking(move || {
            let credentials = CREDENTIALW {
                // SAFETY: takes no arguments and only reads the clock.
                LastWritten: unsafe { GetSystemTimeAsFileTime() },
                Flags: CRED_FLAGS(0),
                Type: CRED_TYPE_GENERIC,
                TargetName: PWSTR::from_raw(target_name.as_mut_ptr()),
                CredentialBlobSize: password.len() as u32,
                CredentialBlob: password.as_ptr() as *mut _,
                Persist: CRED_PERSIST_LOCAL_MACHINE,
                UserName: PWSTR::from_raw(username.as_mut_ptr()),
                ..CREDENTIALW::default()
            };
            // SAFETY: every pointer in `credentials` points into a buffer that outlives the call.
            unsafe {
                CredWriteW(&credentials, 0).map_err(|err| {
                    anyhow!("Failed to write credentials to Windows Credential Manager: {err}")
                })?;
            }
            anyhow::Ok(())
        })
        .await?
    }

    pub(super) async fn read(url: &str) -> Result<Option<(String, Vec<u8>)>> {
        let target_name = target_name(url);
        tokio::task::spawn_blocking(move || {
            let mut credentials: *mut CREDENTIALW = std::ptr::null_mut();
            // SAFETY: the target name is nul-terminated and `credentials` is a valid out-pointer.
            let result = unsafe {
                CredReadW(
                    PCWSTR::from_raw(target_name.as_ptr()),
                    CRED_TYPE_GENERIC,
                    None,
                    &mut credentials,
                )
            };

            if let Err(err) = result {
                // ERROR_NOT_FOUND means the credential doesn't exist.
                // Return Ok(None) to match macOS and Linux behavior.
                if err.code() == ERROR_NOT_FOUND.to_hresult() {
                    return Ok(None);
                }
                return Err(err.into());
            }

            if credentials.is_null() {
                return Ok(None);
            }

            // SAFETY: `CredReadW` succeeded, so this points to a valid `CREDENTIALW` until
            // `CredFree` below.
            let result = unsafe { username_and_password(&*credentials) };
            // SAFETY: `credentials` came from `CredReadW` and is freed once.
            unsafe { CredFree(credentials as *const _ as _) };
            result.map(Some)
        })
        .await?
    }

    pub(super) async fn delete(url: &str) -> Result<()> {
        let target_name = target_name(url);
        tokio::task::spawn_blocking(move || {
            // SAFETY: the target name is nul-terminated.
            unsafe {
                CredDeleteW(
                    PCWSTR::from_raw(target_name.as_ptr()),
                    CRED_TYPE_GENERIC,
                    None,
                )?
            };
            anyhow::Ok(())
        })
        .await?
    }

    /// The nul-terminated target name GPUI stored a credential for `url` under.
    fn target_name(url: &str) -> Vec<u16> {
        format!("zed:url={url}")
            .encode_utf16()
            .chain(Some(0))
            .collect()
    }

    /// # Safety
    ///
    /// The pointers in `credential` must be valid, as `CredReadW` leaves them.
    unsafe fn username_and_password(credential: &CREDENTIALW) -> Result<(String, Vec<u8>)> {
        let username = if credential.UserName.is_null() {
            String::new()
        } else {
            // SAFETY: guaranteed by the caller.
            unsafe { credential.UserName.to_string()? }
        };
        let password = if credential.CredentialBlob.is_null() {
            Vec::new()
        } else {
            // SAFETY: guaranteed by the caller.
            unsafe {
                std::slice::from_raw_parts(
                    credential.CredentialBlob,
                    credential.CredentialBlobSize as usize,
                )
            }
            .to_vec()
        };
        Ok((username, password))
    }
}

#[cfg(not(any(
    target_os = "linux",
    target_os = "freebsd",
    target_os = "macos",
    target_os = "windows"
)))]
mod platform {
    use anyhow::{Result, bail};

    pub(super) async fn write(_: &str, _: &str, _: &[u8]) -> Result<()> {
        bail!("this platform has no keychain")
    }

    pub(super) async fn read(_: &str) -> Result<Option<(String, Vec<u8>)>> {
        Ok(None)
    }

    pub(super) async fn delete(_: &str) -> Result<()> {
        Ok(())
    }
}
