use crate::{Address, B256};

/// A list of addresses and storage keys that the transaction plans to access.
/// Accesses outside the list are possible, but become more expensive.
#[derive(Debug, Clone, Hash, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct AccessListItem {
    /// Account addresses that would be loaded at the start of execution
    pub address: Address,
    /// Keys of storage that would be loaded at the start of execution
    pub storage_keys: Vec<B256>,
}

/// AccessList as defined in EIP-2930
#[derive(Debug, Clone, Hash, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct AccessList(pub Vec<AccessListItem>);

impl From<crate::types::eips::eip2930::AccessList> for AccessList {
    fn from(list: crate::types::eips::eip2930::AccessList) -> Self {
        Self(list.0.into_iter().map(AccessListItem::from).collect())
    }
}

impl From<crate::types::eips::eip2930::AccessListItem> for AccessListItem {
    fn from(item: crate::types::eips::eip2930::AccessListItem) -> Self {
        Self {
            address: item.address,
            storage_keys: item.storage_keys,
        }
    }
}

impl From<AccessListItem> for crate::types::eips::eip2930::AccessListItem {
    fn from(item: AccessListItem) -> Self {
        crate::types::eips::eip2930::AccessListItem {
            address: item.address,
            storage_keys: item.storage_keys,
        }
    }
}

impl From<AccessList> for crate::types::eips::eip2930::AccessList {
    fn from(list: AccessList) -> Self {
        crate::types::eips::eip2930::AccessList(
            list.0.into_iter().map(AccessListItem::into).collect(),
        )
    }
}
