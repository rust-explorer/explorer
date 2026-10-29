app-name = Explorer
loading = Loading…
try-again = Try again
load-failed = Couldn't reach the server.
user-menu-open = Open the user menu
language = Language
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
favorites-title = Favorites
favorites-not-found = Not found
favorites-remove = Remove { $name } from favorites
favorites-load-failed = Favorites couldn't be loaded.
favorites-add = Add to favorites
favorites-remove-current = Remove from favorites
breadcrumb = Breadcrumb
browse-empty = This folder is empty.
browse-empty-hint = Drop files here or use Upload.
browse-empty-hint-touch = Use + to add files.
read-only-notice = Read-only: you can browse, download and copy from this volume, not change it.
read-only-notice-short = Read-only volume
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
error-cross_device = { $path } is on another filesystem than its volume, so nothing can be saved there. An administrator can mount that folder as a volume of its own.
error-invalid_destination = { $path } can't be moved or copied there.
error-task_not_found = This task no longer exists on the server.
error-favorites_full = You can't have more than { $max } favorites. Remove one first.
error-thumbnail_unavailable = No preview is available for this file.
error-too_many_tasks = Too many tasks are waiting. Try again once some have finished.
error-too_many_uploads = You have too many unfinished uploads. Cancel some, or wait a day for the abandoned ones to be cleared.
error-too_many_patches = Too many uploads are being sent at once from your tabs or devices.
error-upload_committed = This upload had already finished, so it could not be cancelled.
error-changed_during_move = { $path } changed while it was being moved, so it was kept in its old place as well as copied to the new one.
error-too_many_entries = { $path } holds too many files and folders to copy or move at once (a million at most). Copy or move its sub-folders one at a time.
error-too_many_zips = Too many zips are being downloaded right now. Try again in a moment.
error-too_many_requests = Too many sign-in attempts from your network. Wait a minute, then try again.
error-selection_expired = This download expired. Select the entries and download them again.
upload = Upload
upload-open = Upload files or a folder
upload-files = Files…
upload-folder = Folder…
upload-read-only = This volume is read-only, so nothing can be uploaded to it.
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
uploads-retrying-server = The server couldn't take it, retrying in { $seconds } s
uploads-invalid-names =
    { $count ->
        [one] 1 item has a name that can't be used here and was skipped.
       *[other] { $count } items have names that can't be used here and were skipped.
    }
uploads-error-permission_denied = You don't have permission to save { $path }.
uploads-error-too-large = The server or a proxy in front of it refused this upload as too large.
conflict-title = { $name } already exists in { $folder }
conflict-replace = Replace
conflict-keep-both = Keep both
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
selection-download-failed = The download could not start.
selection-download-dismiss = Dismiss
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
task-signed-out = Signed out: sign in again to follow this task.
task-cancel-failed = This task couldn't be cancelled.
task-files = { $done } / { $total }
task-files-done =
    { $count ->
        [one] 1 item
       *[other] { $count } items
    }
task-done = Done
task-cancelled = Cancelled
task-vanished = No longer tracked by the server: check the folder to see how it ended.
task-more =
    { $count ->
        [one] and 1 more task
       *[other] and { $count } more tasks
    }
delete-title =
    { $count ->
        [one] Delete { $name }?
       *[other] Delete { $count } items?
    }
delete-warning = Folders are deleted with everything inside. This can't be undone.
delete-confirm = Delete
task-deciding = Already exists: waiting for your answer
task-skipped =
    { $count ->
        [one] 1 item couldn't be copied and was skipped
       *[other] { $count } items couldn't be copied and were skipped
    }
task-skipped-move =
    { $count ->
        [one] 1 item couldn't be moved and stayed in place
       *[other] { $count } items couldn't be moved and stayed in place
    }
copy-title =
    { $count ->
        [one] Copy { $name } to…
       *[other] Copy { $count } items to…
    }
move-title =
    { $count ->
        [one] Move { $name } to…
       *[other] Move { $count } items to…
    }
picker-volume = Volume
picker-choose-volume = Choose a volume
picker-volume-read-only = { $name } (read-only)
picker-location = Destination folder
picker-folders = Folders
picker-empty = No folders here
picker-copy-here = Copy here
picker-move-here = Move here
refresh = Refresh
view-mode = View
view-list = List view
view-grid = Grid view
grid-folder = Folder
grid-count =
    { $count ->
        [one] 1 item
       *[other] { $count } items
    }
search-placeholder = Search in { $volume }
search-open = Search
search-close = Close search
search-searching = Searching…
search-results =
    { $count ->
        [one] 1 result for “{ $query }” in { $volume }
       *[other] { $count } results for “{ $query }” in { $volume }
    }
search-empty = No result for “{ $query }” in { $volume }
search-truncated = Showing the first { $count }, refine your search.
search-timed-out = The search took too long; results may be incomplete.
action-analyze-usage = Analyse space
action-calculate-size = Calculate size
analyze-usage = Analyse space
folder-size-calculated = Calculated at { $time }
folder-size-computing = Calculating size…
task-analysing = Analysing { $name }
task-analysing-counts =
    { $count ->
        [one] Analysing { $name } — { $files } file, { $size }
       *[other] Analysing { $name } — { $files } files, { $size }
    }
task-view = View
task-view-usage = View the space used by { $name }
usage-breadcrumb = Scanned folders
usage-items = Contents
usage-empty = This folder is empty.
usage-files =
    { $count ->
        [one] { $files } file
       *[other] { $files } files
    }
usage-folders =
    { $count ->
        [one] { $folders } folder
       *[other] { $folders } folders
    }
usage-other-files =
    { $count ->
        [one] { $files } other file
       *[other] { $files } other files
    }
usage-percent = { $percent }%
usage-percent-small = <1%
usage-scanned-at = Scanned at { $time }
usage-scan-again = Scan again
usage-open-in-explorer = Open in explorer
usage-incomplete = Some items could not be read
usage-collapsed = Too many folders: sub-folders of this one are counted but not listed
usage-scanning = Analysing { $name }…
usage-cancelled-title = Scan cancelled
usage-cancelled-message = This scan was stopped before it finished.
usage-unavailable-title = This scan is no longer available
usage-unavailable-message = Results are only kept for a while after a scan. Scan the folder again to see what takes up its space.
usage-failed-title = The scan failed
usage-scan-finished = Scan finished
