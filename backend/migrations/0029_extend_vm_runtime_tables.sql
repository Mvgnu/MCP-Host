-- key: migration -> runtime-vm-metadata
ALTER TABLE runtime_vm_instances
    ADD COLUMN IF NOT EXISTS attestation_hint JSONB;
ALTER TABLE runtime_vm_instances
    ADD COLUMN IF NOT EXISTS hypervisor_endpoint TEXT;
ALTER TABLE runtime_vm_instances
    ADD COLUMN IF NOT EXISTS hypervisor_credentials JSONB;
ALTER TABLE runtime_vm_instances
    ADD COLUMN IF NOT EXISTS hypervisor_network_template JSONB;
ALTER TABLE runtime_vm_instances
    ADD COLUMN IF NOT EXISTS hypervisor_volume_template JSONB;
ALTER TABLE runtime_vm_instances
    ADD COLUMN IF NOT EXISTS gpu_passthrough_policy JSONB;
