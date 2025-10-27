# Backend Runtime Notes

## Libvirt Executor Configuration
The runtime can now launch confidential VMs directly against libvirt. The executor reads its configuration from environment variables so deployments can switch drivers without rebuilding the binary. Key variables include:

| Variable | Description |
| --- | --- |
| `VM_PROVISIONER_DRIVER` | Set to `libvirt` to enable the libvirt executor, or leave as `http` to use the legacy hypervisor API. |
| `LIBVIRT_CONNECTION_URI` | Libvirt connection URI (for example `qemu:///system` or `qemu+ssh://host/system`). |
| `LIBVIRT_USERNAME` / `LIBVIRT_PASSWORD` | Optional credentials used during connection negotiation. Passwords can also be supplied via `LIBVIRT_PASSWORD_FILE`. |
| `LIBVIRT_DEFAULT_ISOLATION_TIER` | Default isolation tier applied when policy decisions omit an explicit value. |
| `LIBVIRT_DEFAULT_MEMORY_MIB` / `LIBVIRT_DEFAULT_VCPU_COUNT` | Baseline VM sizing in MiB and vCPU count. |
| `LIBVIRT_LOG_TAIL` | Number of console lines fetched per request; affects the HTTP API and CLI log views. |
| `LIBVIRT_NETWORK_TEMPLATE` | JSON template merged into the generated domain XML network stanza. |
| `LIBVIRT_VOLUME_TEMPLATE` | JSON template describing the boot volume path, driver, and target bus. |
| `LIBVIRT_GPU_POLICY` | JSON policy describing GPU passthrough requirements (devices, enablement flag). |
| `LIBVIRT_CONSOLE_SOURCE` | Optional console source (`pty`, `tcp`, etc.) inserted into the domain XML. |

All JSON templates are validated during startup. Invalid values will cause the process to abort with an explanatory panic so operators can fix misconfigurations early.

## Deployment Prerequisites
Deployments must install the libvirt daemon and ensure the runtime has permission to create and control domains. On Linux hosts this typically requires adding the service account to the `libvirt` group and enabling `libvirtd` + `virtlogd`. When running over SSH (`qemu+ssh://`) make sure host keys are trusted and the runtime user can read any required private keys.

Enabling the libvirt executor also requires building the backend with the `libvirt-executor` cargo feature so the native driver is compiled in.

## Secret Rotation
Passwords can be rotated without restarts by updating `LIBVIRT_PASSWORD_FILE` to point at the new secret and sending a SIGHUP (or restarting the process). The runtime records only sanitized snapshots of credentials in `runtime_vm_instances`, ensuring API consumers see whether secrets were supplied without exposing raw values.

## Troubleshooting Console Access
If log retrieval returns empty output, verify `LIBVIRT_CONSOLE_SOURCE` matches the domain's serial device configuration and that `LIBVIRT_LOG_TAIL` is large enough to capture recent lines. The new streaming test covers channel setup end-to-end; if streaming fails in production, confirm `virtlogd` is running and that SELinux/AppArmor policies allow read access to the console device.
