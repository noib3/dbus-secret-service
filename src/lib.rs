// Copyright 2016-2024 dbus-secret-service Contributors
//
// Licensed under the Apache License, Version 2.0, <LICENSE-APACHE or
// http://apache.org/licenses/LICENSE-2.0> or the MIT license <LICENSE-MIT or
// http://opensource.org/licenses/MIT>, at your option. This file may not be
// copied, modified, or distributed except according to those terms.

//! # Secret Service library
//!
//! This library implements a rust interface to the Secret Service API which is implemented
//! in Linux.
//!
//! ## About Secret Service API
//! <https://standards.freedesktop.org/secret-service/>
//!
//! Secret Service provides a secure place to store secrets.
//! Gnome keyring and KWallet implement the Secret Service API.
//!
//! ## Basic Usage
//! ```
//! use dbus_secret_service::SecretService;
//! use dbus_secret_service::EncryptionType;
//! use std::collections::HashMap;
//!
//! fn main() {
//!    // initialize secret service (dbus connection and encryption session)
//!    let ss = SecretService::connect(EncryptionType::Dh).unwrap();
//!
//!    // get default collection
//!    let collection = ss.get_default_collection().unwrap();
//!
//!    let mut properties = HashMap::new();
//!    properties.insert("test", "test_value");
//!
//!    //create new item
//!    collection.create_item(
//!        "test_label", // label
//!        properties,
//!        b"test_secret", //secret
//!        false, // replace item with same attributes
//!        "text/plain" // secret content type
//!    ).unwrap();
//!
//!    // search items by properties
//!    let search_items = ss.search_items(
//!        HashMap::from([("test", "test_value")])
//!    ).unwrap();
//!
//!    // retrieve one item, first by checking the unlocked items
//!    let item = match search_items.unlocked.first() {
//!        Some(item) => item,
//!        None => {
//!            // if there aren't any, check the locked items and unlock the first one
//!            let locked_item = search_items
//!                .locked
//!                .first()
//!                .expect("Search didn't return any items!");
//!            locked_item.unlock().unwrap();
//!            locked_item
//!        }
//!    };
//!
//!    // retrieve secret from item
//!    let secret = item.get_secret().unwrap();
//!    assert_eq!(secret, b"test_secret");
//!
//!    // delete item (deletes the dbus object, not the struct instance)
//!    item.delete().unwrap()
//! }
//! ```
//!
//! ## Overview of this library:
//! ### Entry point
//! The entry point for this library is the `SecretService` struct. A new instance of
//! `SecretService` will initialize the dbus connection and negotiate an encryption session.
//!
//! ```
//! # use dbus_secret_service::SecretService;
//! # use dbus_secret_service::EncryptionType;
//! # fn call() {
//! SecretService::connect(EncryptionType::Plain).unwrap();
//! # }
//! ```
//!
//! or
//!
//! ```
//! # use dbus_secret_service::SecretService;
//! # use dbus_secret_service::EncryptionType;
//! # fn call() {
//! SecretService::connect(EncryptionType::Dh).unwrap();
//! # }
//! ```
//!
//! Once the SecretService struct is initialized, it can be used to navigate to a collection.
//! Items can also be directly searched for without getting a collection first.
//!
//! ### Collections and Items
//! The Secret Service API organizes secrets into collections, and holds each secret
//! in an item.
//!
//! Items consist of a label, attributes, and the secret. The most common way to find
//! an item is a search by attributes.
//!
//! While it's possible to create new collections, most users will simply create items
//! within the default collection.
//!
//! ### Actions overview
//! The most common supported actions are `create`, `get`, `search`, and `delete` for
//! `Collections` and `Items`. For more specifics and exact method names, please see
//! each structure's documentation.
//!
//! In addition, `set` and `get` actions are available for secrets contained in an `Item`.
//!
//! ### Crypto
//! Specifics in SecretService API Draft Proposal:
//! <https://standards.freedesktop.org/secret-service/>
//!

use std::collections::HashMap;

pub use collection::Collection;
use dbus::arg::RefArg;
use dbus::{
    arg::{PropMap, Variant},
    blocking::{Connection, Proxy},
    strings::Path,
};
pub use error::Error;
pub use item::Item;
use proxy::{new_proxy, service::Service};
pub use session::EncryptionType;
use session::Session;
use ss::{SS_COLLECTION_LABEL, SS_DBUS_PATH};

mod collection;
mod error;
mod item;
mod prompt;
mod proxy;
mod session;
mod ss;

/// Secret Service Struct.
///
/// This the main entry point for usage of the library.
///
/// Creating a new [SecretService] will also initialize dbus
/// and negotiate a new cryptographic session
/// ([EncryptionType::Plain] or [EncryptionType::Dh])
pub struct SecretService {
    connection: Connection,
    session: Session,
    timeout: Option<u64>,
}

/// Used to indicate locked and unlocked items in the
/// return value of [SecretService::search_items]
/// and [blocking::SecretService::search_items].
pub struct SearchItemsResult<T> {
    pub unlocked: Vec<T>,
    pub locked: Vec<T>,
}

pub(crate) enum LockAction {
    Lock,
    Unlock,
}

impl SecretService {
    /// Connect to the DBus and return a new [SecretService] instance.
    pub fn connect(encryption: EncryptionType) -> Result<Self, Error> {
        let connection = Connection::new_session()?;
        let session = Session::new(new_proxy(&connection, SS_DBUS_PATH), encryption)?;
        Ok(SecretService {
            connection,
            session,
            timeout: None,
        })
    }

    /// Connect to the DBus and return a new [SecretService] instance.
    ///
    /// Instead of waiting indefinitely for users to respond to prompts,
    /// this instance will time them out after a given number of seconds.
    /// (Specifying 0 for the number of seconds will prevent the prompt
    /// from appearing at all.)
    pub fn connect_with_max_prompt_timeout(
        encryption: EncryptionType,
        seconds: u64,
    ) -> Result<Self, Error> {
        let mut service = Self::connect(encryption)?;
        service.timeout = Some(seconds);
        Ok(service)
    }

    /// Get the service proxy (internal)
    fn proxy<'a>(&'a self) -> Proxy<'a, &'a Connection> {
        new_proxy(&self.connection, SS_DBUS_PATH)
    }

    /// Get all collections
    pub fn get_all_collections(&self) -> Result<Vec<Collection>, Error> {
        let paths = self.proxy().collections()?;
        let collections = paths
            .into_iter()
            .map(|path| Collection::new(self, path))
            .collect();
        Ok(collections)
    }

    /// Get collection by alias.
    ///
    /// Most common would be the `default` alias, but there
    /// is also a specific method for getting the collection
    /// by default alias.
    pub fn get_collection_by_alias(&self, alias: &str) -> Result<Collection, Error> {
        let path = self.proxy().read_alias(alias)?;
        if path == Path::new("/")? {
            Err(Error::NoResult)
        } else {
            Ok(Collection::new(self, path))
        }
    }

    /// Get default collection.
    /// (The collection whose alias is `default`)
    pub fn get_default_collection(&self) -> Result<Collection<'_>, Error> {
        self.get_collection_by_alias("default")
    }

    /// Get any collection.
    /// First tries `default` collection, then `session`
    /// collection, then the first collection when it
    /// gets all collections.
    pub fn get_any_collection(&self) -> Result<Collection<'_>, Error> {
        self.get_default_collection()
            .or_else(|_| self.get_collection_by_alias("session"))
            .or_else(|_| {
                let mut collections = self.get_all_collections()?;
                if collections.is_empty() {
                    Err(Error::NoResult)
                } else {
                    Ok(collections.swap_remove(0))
                }
            })
    }

    /// Creates a new collection with a label and an alias.
    pub fn create_collection(&self, label: &str, alias: &str) -> Result<Collection<'_>, Error> {
        let mut properties: PropMap = HashMap::new();
        properties.insert(
            SS_COLLECTION_LABEL.to_string(),
            Variant(Box::new(label.to_string()) as Box<dyn RefArg>),
        );
        // create collection returning collection path and prompt path
        let (c_path, p_path) = self.proxy().create_collection(properties, alias)?;
        let created = {
            if c_path == Path::new("/")? {
                // no creation path, so prompt
                self.prompt_for_create(&p_path)?
            } else {
                c_path
            }
        };
        Ok(Collection::new(self, created))
    }

    /// Searches all items by attributes
    pub fn search_items(
        &self,
        attributes: HashMap<&str, &str>,
    ) -> Result<SearchItemsResult<Item<'_>>, Error> {
        let (unlocked, locked) = self.proxy().search_items(attributes)?;
        let result = SearchItemsResult {
            unlocked: unlocked.into_iter().map(|p| Item::new(self, p)).collect(),
            locked: locked.into_iter().map(|p| Item::new(self, p)).collect(),
        };
        Ok(result)
    }

    /// Unlock all items in a batch
    pub fn unlock_all(&self, items: &[&Item<'_>]) -> Result<(), Error> {
        let paths = items.iter().map(|i| i.path.clone()).collect();
        self.lock_unlock_all(LockAction::Unlock, paths)
    }

    pub(crate) fn lock_unlock_all(
        &self,
        action: LockAction,
        paths: Vec<Path>,
    ) -> Result<(), Error> {
        let (_, p_path) = match action {
            LockAction::Lock => self.proxy().lock(paths)?,
            LockAction::Unlock => self.proxy().unlock(paths)?,
        };
        if p_path == Path::new("/")? {
            Ok(())
        } else {
            self.prompt_for_lock_unlock_delete(&p_path)
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn should_create_secret_service() {
        SecretService::connect(EncryptionType::Plain).unwrap();
    }

    #[test]
    fn should_get_all_collections() {
        // Assumes that there will always be a default collection
        let ss = SecretService::connect(EncryptionType::Plain).unwrap();
        let collections = ss.get_all_collections().unwrap();
        assert!(!collections.is_empty(), "no collections found");
    }

    #[test]
    fn should_get_collection_by_alias() {
        let ss = SecretService::connect(EncryptionType::Plain).unwrap();
        ss.get_collection_by_alias("session").unwrap();
    }

    #[test]
    fn should_return_error_if_collection_doesnt_exist() {
        let ss = SecretService::connect(EncryptionType::Plain).unwrap();

        match ss.get_collection_by_alias("definitely_definitely_does_not_exist") {
            Err(Error::NoResult) => {}
            _ => panic!(),
        };
    }

    #[test]
    fn should_get_default_collection() {
        let ss = SecretService::connect(EncryptionType::Plain).unwrap();
        ss.get_default_collection().unwrap();
    }

    #[test]
    fn should_get_any_collection() {
        let ss = SecretService::connect(EncryptionType::Plain).unwrap();
        let _ = ss.get_any_collection().unwrap();
    }

    #[test_with::no_env(GITHUB_ACTIONS)] // can't run headless - prompts
    fn should_create_and_delete_collection() {
        let ss = SecretService::connect(EncryptionType::Plain).unwrap();
        let test_collection = ss.create_collection("TestCreateDelete", "").unwrap();
        assert!(test_collection
            .path
            .starts_with("/org/freedesktop/secrets/collection/Test"));
        test_collection.delete().unwrap();
    }

    #[test]
    fn should_search_items() {
        let ss = SecretService::connect(EncryptionType::Plain).unwrap();
        let collection = ss.get_default_collection().unwrap();

        // Create an item
        let item = collection
            .create_item(
                "test",
                HashMap::from([("test_attribute_in_ss", "test_value")]),
                b"test_secret",
                false,
                "text/plain",
            )
            .unwrap();

        // handle empty vec search
        ss.search_items(HashMap::new()).unwrap();

        // handle no result
        let bad_search = ss.search_items(HashMap::from([("test", "test")])).unwrap();
        assert_eq!(bad_search.unlocked.len(), 0);
        assert_eq!(bad_search.locked.len(), 0);

        // handle correct search for item and compare
        let search_item = ss
            .search_items(HashMap::from([("test_attribute_in_ss", "test_value")]))
            .unwrap();

        assert_eq!(item.path, search_item.unlocked[0].path);
        assert_eq!(search_item.locked.len(), 0);
        item.delete().unwrap();
    }

    #[test_with::no_env(GITHUB_ACTIONS)] // can't run headless - prompts
    fn should_lock_and_unlock() {
        // Assumes that there will always be at least one collection
        let ss = SecretService::connect(EncryptionType::Plain).unwrap();
        let collections = ss.get_all_collections().unwrap();
        assert!(!collections.is_empty(), "no collections found");
        let paths: Vec<Path> = collections.iter().map(|c| c.path.clone()).collect();
        ss.lock_unlock_all(LockAction::Lock, paths.clone()).unwrap();
        ss.lock_unlock_all(LockAction::Unlock, paths).unwrap();
    }
}
