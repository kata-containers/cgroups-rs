// Copyright (c) 2025 Ant Group
//
// SPDX-License-Identifier: Apache-2.0 or MIT
//

use zbus::blocking::Connection;
use zbus::Result;

use crate::systemd::dbus::systemd_manager_proxy::ManagerProxyBlocking as SystemManager;

pub(crate) fn systemd_manager_proxy<'a>() -> Result<SystemManager<'a>> {
    let connection = Connection::system()?;
    let proxy = SystemManager::new(&connection)?;

    Ok(proxy)
}
