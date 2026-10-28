app-name = Explorer
loading = Cargando…
try-again = Volver a intentarlo
load-failed = No se pudo conectar con el servidor.
user-menu-open = Abrir el menú de usuario
user-menu-language = Idioma
sign-out = Cerrar sesión
sign-out-failed = No se pudo cerrar la sesión. Vuelve a intentarlo.
access-denied-title = Acceso denegado
access-denied-message = Tu cuenta no tiene permiso para usar Explorer. Pide a un administrador que te dé acceso o inicia sesión con otra cuenta.
signin-error-title = No se pudo iniciar sesión
signin-error-provider-error = El proveedor de identidad rechazó el inicio de sesión.
signin-error-state-mismatch = La solicitud de inicio de sesión caducó o se abrió en otra pestaña.
signin-error-exchange-failed = Explorer no pudo completar el inicio de sesión con el proveedor de identidad.
signin-error-invalid-token = El proveedor de identidad envió una respuesta no válida.
signin-error-missing-groups-claim = Tu cuenta no incluye información de grupos, así que no se puede comprobar tu acceso. Pide a un administrador que revise la configuración del proveedor de identidad.
signin-error-unknown = Algo salió mal al iniciar sesión.
not-found-title = Página no encontrada
not-found-message = No hay nada en esta dirección.
no-volumes-title = No hay volúmenes
no-volumes-message = Explorer todavía no tiene ningún volumen que mostrar. Pide a un administrador que añada una carpeta al directorio de datos.
sidebar-open = Abrir la lista de volúmenes
sidebar-volumes = Volúmenes
sidebar-read-only = Solo lectura
sidebar-free = { $volume } · { $available } libres
sidebar-used-label = Espacio usado
sidebar-used = { $used } usados de { $total }
favorites-title = Favoritos
favorites-not-found = No encontrada
favorites-remove = Quitar { $name } de favoritos
favorites-load-failed = No se pudieron cargar los favoritos.
favorites-add = Añadir a favoritos
favorites-remove-current = Quitar de favoritos
breadcrumb = Ruta de navegación
browse-empty = Esta carpeta está vacía.
browse-empty-hint = Suelta archivos aquí o usa Subir.
read-only-notice = Solo lectura: puedes explorar, descargar y copiar desde este volumen, pero no modificarlo.
read-only-notice-short = Volumen de solo lectura
table-label = Contenido de la carpeta
table-name = Nombre
table-size = Tamaño
table-modified = Modificado
table-actions = Acciones
modified-yesterday = ayer
download-file = Descargar { $name }
download-folder = Descargar { $name } como zip
entry-unavailable = Este elemento no se puede abrir.
error-reference = Referencia: { $id }
error-open-volume = Abrir { $volume }
error-go-home = Ir a la página de inicio
error-unknown = Algo salió mal. Vuelve a intentarlo en un momento.
error-volume_not_found = El volumen { $path } no existe.
error-path_not_found = { $path } no existe. Puede que se haya movido o eliminado.
error-invalid_path = Esta dirección no apunta a una ubicación válida.
error-not_a_directory = { $path } es un archivo, no una carpeta.
error-is_a_directory = { $path } es una carpeta, no un archivo.
error-permission_denied = No tienes permiso para abrir { $path }.
error-outside_volume = { $path } apunta fuera de su volumen, así que no se puede abrir.
error-range_not_satisfiable = La parte solicitada del archivo está fuera de rango.
error-internal = Ha ocurrido un error en el servidor. Inténtalo de nuevo en un momento.
error-forbidden = No tienes acceso a este recurso.
error-already_exists = { $path } ya existe.
error-read_only_volume = El volumen { $path } es de solo lectura.
error-insufficient_storage = No hay suficiente espacio libre para guardar { $path }.
error-upload_not_found = Esta subida ya no existe en el servidor.
error-offset_mismatch = La subida se ha desincronizado del servidor.
error-upload_busy = Esta subida ya se está enviando desde otra pestaña u otro dispositivo.
error-upload_too_large = Este archivo es demasiado grande para subirlo.
error-invalid_upload = El servidor ha rechazado esta subida.
error-unsupported_tus_version = El servidor no admite esta versión del protocolo de subida.
error-unsupported_media_type = El servidor ha rechazado el formato de esta subida.
error-cross_device = { $path } está en otro sistema de archivos que su volumen, así que no se puede guardar nada ahí. Un administrador puede montar esa carpeta como un volumen propio.
error-invalid_destination = { $path } no se puede mover ni copiar ahí.
error-task_not_found = Esta tarea ya no existe en el servidor.
error-favorites_full = No puedes tener más de { $max } favoritos. Quita uno primero.
error-thumbnail_unavailable = No hay vista previa disponible para este archivo.
upload = Subir
upload-open = Subir archivos o una carpeta
upload-files = Archivos…
upload-folder = Carpeta…
upload-read-only = Este volumen es de solo lectura, así que no se puede subir nada.
uploads-cancel = Cancelar { $name }
uploads-progress = Progreso de la subida
uploads-waiting = En espera
uploads-needs-decision = Esperando tu elección
uploads-creating-folder = Creando la carpeta…
uploads-sent = { $sent } / { $total }
uploads-resumable = reanudable
uploads-retrying = Conexión perdida, nuevo intento en { $seconds } s
uploads-done = Terminada
uploads-cancelled = Cancelada
uploads-skipped = Omitida
uploads-unreadable =
    { $count ->
        [one] 1 elemento no se pudo leer y se omitió.
       *[other] { $count } elementos no se pudieron leer y se omitieron.
    }
uploads-retrying-server = El servidor no pudo aceptarla, nuevo intento en { $seconds } s
uploads-invalid-names =
    { $count ->
        [one] 1 elemento tiene un nombre que no se puede usar aquí y se omitió.
       *[other] { $count } elementos tienen nombres que no se pueden usar aquí y se omitieron.
    }
uploads-error-permission_denied = No tienes permiso para guardar { $path }.
uploads-error-too-large = El servidor o un proxy delante de él rechazó esta subida por ser demasiado grande.
conflict-title = { $name } ya existe en { $folder }
conflict-replace = Reemplazar
conflict-keep-both = Conservar ambos
conflict-apply-all = Aplicar a todos
upload-drop = Suelta para subir a { $folder }
new-folder = Nueva carpeta
new-folder-title = Nueva carpeta
new-folder-name = Nombre
new-folder-create = Crear
cancel = Cancelar
name-error-empty = Introduce un nombre.
name-error-slash = Un nombre no puede contener barras.
name-error-dot = Un nombre no puede ser . ni ..
name-error-reserved = Ese nombre está reservado.
name-error-too-long = Ese nombre es demasiado largo (máximo 255 bytes).
row-menu-open = Acciones para { $name }
action-download = Descargar
action-rename = Cambiar nombre
action-move-to = Mover a…
action-copy-to = Copiar a…
action-delete = Eliminar
select-all = Seleccionar todas las entradas
select-entry = Seleccionar { $name }
selection-count =
    { $count ->
        [one] { $count } seleccionado
       *[other] { $count } seleccionados
    }
selection-clear = Borrar selección
rename-title = Cambiar el nombre de { $name }
rename-name = Nuevo nombre
rename-save = Guardar
activity-title =
    { $count ->
        [0] Actividad
        [one] Actividad · 1 en curso
       *[other] Actividad · { $count } en curso
    }
activity-collapse = Contraer el panel de actividad
activity-expand = Expandir el panel de actividad
activity-close = Cerrar el panel de actividad
task-deleting = Eliminando { $name }
task-copying = Copiando { $name } a { $folder }
task-moving = Moviendo { $name } a { $folder }
task-cancel = Cancelar { $name }
task-progress = Progreso de la tarea
task-waiting = En espera
task-signed-out = Sesión cerrada: vuelve a iniciar sesión para seguir esta tarea.
task-cancel-failed = No se pudo cancelar esta tarea.
task-files = { $done } / { $total }
task-files-done =
    { $count ->
        [one] 1 elemento
       *[other] { $count } elementos
    }
task-done = Terminada
task-cancelled = Cancelada
delete-title =
    { $count ->
        [one] ¿Eliminar { $name }?
       *[other] ¿Eliminar { $count } elementos?
    }
delete-warning = Las carpetas se eliminan con todo su contenido. Esta acción no se puede deshacer.
delete-confirm = Eliminar
task-deciding = Ya existe: esperando tu respuesta
task-skipped =
    { $count ->
        [one] 1 elemento no se pudo copiar y se omitió
       *[other] { $count } elementos no se pudieron copiar y se omitieron
    }
task-skipped-move =
    { $count ->
        [one] 1 elemento no se pudo mover y se quedó en su sitio
       *[other] { $count } elementos no se pudieron mover y se quedaron en su sitio
    }
copy-title =
    { $count ->
        [one] Copiar { $name } a…
       *[other] Copiar { $count } elementos a…
    }
move-title =
    { $count ->
        [one] Mover { $name } a…
       *[other] Mover { $count } elementos a…
    }
picker-volume = Volumen
picker-choose-volume = Elige un volumen
picker-volume-read-only = { $name } (solo lectura)
picker-location = Carpeta de destino
picker-folders = Carpetas
picker-empty = No hay carpetas aquí
picker-copy-here = Copiar aquí
picker-move-here = Mover aquí
refresh = Actualizar
view-mode = Vista
view-list = Vista de lista
view-grid = Vista de cuadrícula
grid-folder = Carpeta
grid-count =
    { $count ->
        [one] 1 elemento
       *[other] { $count } elementos
    }
search-placeholder = Buscar en { $volume }
search-open = Buscar
search-close = Cerrar la búsqueda
search-searching = Buscando…
search-results =
    { $count ->
        [one] 1 resultado para «{ $query }» en { $volume }
       *[other] { $count } resultados para «{ $query }» en { $volume }
    }
search-empty = Ningún resultado para «{ $query }» en { $volume }
search-truncated = Se muestran los { $count } primeros, afina la búsqueda.
search-timed-out = La búsqueda tardó demasiado; puede que falten resultados.
action-analyze-usage = Analizar espacio
action-calculate-size = Calcular tamaño
analyze-usage = Analizar espacio
folder-size-calculated = Calculado a las { $time }
folder-size-computing = Calculando el tamaño…
task-analysing = Analizando { $name }
task-analysing-counts =
    { $count ->
        [one] Analizando { $name } — { $files } archivo, { $size }
       *[other] Analizando { $name } — { $files } archivos, { $size }
    }
task-view = Ver
task-view-usage = Ver el espacio que ocupa { $name }
usage-breadcrumb = Carpetas analizadas
usage-items = Contenido
usage-empty = Esta carpeta está vacía.
usage-files =
    { $count ->
        [one] { $files } archivo
       *[other] { $files } archivos
    }
usage-folders =
    { $count ->
        [one] { $folders } carpeta
       *[other] { $folders } carpetas
    }
usage-other-files =
    { $count ->
        [one] { $files } archivo más
       *[other] { $files } archivos más
    }
usage-percent = { $percent } %
usage-percent-small = < 1 %
usage-scanned-at = Analizado a las { $time }
usage-scan-again = Volver a analizar
usage-open-in-explorer = Abrir en el explorador
usage-incomplete = Algunos elementos no se pudieron leer
usage-collapsed = Demasiadas carpetas: las subcarpetas de esta se cuentan, pero no se listan
usage-scanning = Analizando { $name }…
usage-cancelled-title = Análisis cancelado
usage-cancelled-message = Este análisis se detuvo antes de terminar.
usage-unavailable-title = Este análisis ya no está disponible
usage-unavailable-message = Los resultados solo se conservan un tiempo tras el análisis. Vuelve a analizar la carpeta para ver qué ocupa su espacio.
usage-failed-title = El análisis ha fallado
usage-scan-finished = Análisis terminado
