# Systemd Dbus

How to generate `xxx_proxy.rs` files

```shell
# install zbus-xmlgen if not
$ cargo install zbus-xmlgen
# generate interface in XML format
$ busctl introspect --xml-interface \
    org.freedesktop.systemd1 \
    /org/freedesktop/systemd1 \
    org.freedesktop.systemd1.Manager > /tmp/systemd1-manager.xml
# generate Rust code from XML
$ zbus-xmlgen file /tmp/systemd1-manager.xml \
    --output src/systemd/dbus/systemd_manager_proxy.rs
$ rm -rf /tmp/systemd1-manager.xml
```
