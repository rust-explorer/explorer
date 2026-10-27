app-name = Explorer
loading = Loading…
try-again = Try again
load-failed = Couldn't reach the server.
user-menu-open = Open the user menu
user-menu-language = Language
sign-out = Sign out
sign-out-failed = Couldn't sign you out. Try again.
access-denied-title = Access denied
access-denied-message = Your account isn't allowed to use Explorer. Ask an administrator to add you, or sign in with another account.
signin-error-title = Sign-in failed
signin-error-provider-error = The identity provider refused the sign-in.
signin-error-state-mismatch = The sign-in request expired or was opened in another tab.
signin-error-exchange-failed = Explorer couldn't complete the sign-in with the identity provider.
signin-error-invalid-token = The identity provider sent an invalid response.
signin-error-missing-groups-claim = Your account has no group information, so your access can't be checked. Ask an administrator to check the identity provider configuration.
signin-error-unknown = Something went wrong during sign-in.
not-found-title = Page not found
not-found-message = There's nothing at this address.
no-volumes-title = No volumes
no-volumes-message = Explorer doesn't have any volume to show yet. Ask an administrator to add a folder to the data directory.
sidebar-open = Open the volume list
sidebar-volumes = Volumes
sidebar-read-only = Read-only
sidebar-free = { $volume } · { $available } free
sidebar-used-label = Used space
sidebar-used = { $used } used of { $total }
breadcrumb = Breadcrumb
browse-empty = This folder is empty.
table-label = Folder contents
table-name = Name
table-size = Size
table-modified = Modified
table-actions = Actions
modified-yesterday = Yesterday
download-file = Download { $name }
download-folder = Download { $name } as a zip
entry-unavailable = This item can't be opened.
error-reference = Reference: { $id }
error-open-volume = Open { $volume }
error-go-home = Go to the home page
error-unknown = Something went wrong. Try again in a moment.
error-volume_not_found = The volume { $path } doesn't exist.
error-path_not_found = { $path } doesn't exist. It may have been moved or deleted.
error-invalid_path = This address doesn't point to a valid location.
error-not_a_directory = { $path } is a file, not a folder.
error-is_a_directory = { $path } is a folder, not a file.
error-permission_denied = You don't have permission to open { $path }.
error-outside_volume = { $path } points outside its volume, so it can't be opened.
error-range_not_satisfiable = The requested part of the file is out of range.
error-internal = Something went wrong on the server. Try again in a moment.
error-forbidden = You don't have access to this.
error-already_exists = { $path } already exists.
error-read_only_volume = The volume { $path } is read-only.
error-insufficient_storage = There isn't enough free space to save { $path }.
error-upload_not_found = This upload no longer exists on the server.
error-offset_mismatch = The upload got out of step with the server.
error-upload_busy = This upload is already being sent from another tab or device.
error-upload_too_large = This file is too large to upload.
error-invalid_upload = The server refused this upload.
error-unsupported_tus_version = The server doesn't support this upload protocol version.
error-unsupported_media_type = The server refused the format of this upload.
error-cross_device = { $path } is on another filesystem than its volume, so the upload can't be saved there. An administrator can mount that folder as a volume of its own.
error-invalid_destination = { $path } can't be moved or copied there.
error-task_not_found = This task no longer exists on the server.
upload = Upload
upload-open = Upload files or a folder
upload-files = Files…
upload-folder = Folder…
upload-read-only = This volume is read-only, so nothing can be uploaded to it.
uploads-title = Uploads ({ $count })
uploads-collapse = Collapse the uploads
uploads-expand = Expand the uploads
uploads-close = Close the uploads
uploads-cancel = Cancel { $name }
uploads-progress = Upload progress
uploads-waiting = Waiting
uploads-needs-decision = Waiting for your choice
uploads-creating-folder = Creating the folder…
uploads-sent = { $sent } / { $total }
uploads-resumable = resumable
uploads-retrying = Connection lost, retrying in { $seconds } s
uploads-done = Done
uploads-cancelled = Cancelled
uploads-skipped = Skipped
uploads-unreadable =
    { $count ->
        [one] 1 item couldn't be read and was skipped.
       *[other] { $count } items couldn't be read and were skipped.
    }
conflict-title = { $name } already exists in { $folder }
conflict-replace = Replace
conflict-keep-both = Keep both
conflict-cancel = Cancel
conflict-apply-all = Apply to all
upload-drop = Drop to upload to { $folder }
new-folder = New folder
new-folder-title = New folder
new-folder-name = Name
new-folder-create = Create
cancel = Cancel
name-error-empty = Enter a name.
name-error-slash = A name can't contain a slash.
name-error-dot = A name can't be . or ..
name-error-reserved = That name is reserved.
name-error-too-long = That name is too long (255 bytes at most).
row-menu-open = Actions for { $name }
action-download = Download
action-rename = Rename
action-move-to = Move to…
action-copy-to = Copy to…
action-delete = Delete
select-all = Select all entries
select-entry = Select { $name }
selection-count = { $count } selected
selection-clear = Clear selection
rename-title = Rename { $name }
rename-name = New name
rename-save = Save
activity-title =
    { $count ->
        [0] Activity
        [one] Activity · 1 in progress
       *[other] Activity · { $count } in progress
    }
activity-collapse = Collapse the activity panel
activity-expand = Expand the activity panel
activity-close = Close the activity panel
task-deleting = Deleting { $name }
task-copying = Copying { $name } to { $folder }
task-moving = Moving { $name } to { $folder }
task-cancel = Cancel { $name }
task-progress = Task progress
task-waiting = Waiting
task-files = { $done } / { $total }
task-files-done =
    { $count ->
        [one] 1 item
       *[other] { $count } items
    }
task-done = Done
task-cancelled = Cancelled
delete-title =
    { $count ->
        [one] Delete { $name }?
       *[other] Delete { $count } items?
    }
delete-warning = Folders are deleted with everything inside. This can't be undone.
delete-confirm = Delete
