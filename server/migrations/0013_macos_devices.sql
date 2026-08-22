-- 0013_macos_devices — the Mac is a third kind of device.
--
-- A native macOS build of this app shares the iOS bundle id, so its APNs
-- topic is the same one and its tokens go out over the same token-based
-- HTTP/2 connection. Nothing about DELIVERY changes; what changes is that
-- the server can no longer pretend a Mac is an iPhone.
--
-- It matters because `platform` is what routes a device to APNs or FCM
-- (push.rs). A Mac registering honestly as 'macos' against the old CHECK
-- would be refused at the door; registering as 'ios' would work and then
-- lie in every row, which is the kind of small untruth that costs an hour
-- the first time somebody debugs "why did only one device get it".

ALTER TABLE devices DROP CONSTRAINT devices_platform_check;
ALTER TABLE devices ADD CONSTRAINT devices_platform_check
    CHECK (platform IN ('ios', 'android', 'macos'));
