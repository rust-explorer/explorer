app-name = Explorer
loading = Chargement…
try-again = Réessayer
load-failed = Impossible de joindre le serveur.
user-menu-open = Ouvrir le menu utilisateur
user-menu-language = Langue
sign-out = Se déconnecter
sign-out-failed = La déconnexion a échoué. Réessayez.
access-denied-title = Accès refusé
access-denied-message = Votre compte n'est pas autorisé à utiliser Explorer. Demandez à un administrateur de vous ajouter, ou connectez-vous avec un autre compte.
signin-error-title = Échec de la connexion
signin-error-provider-error = Le fournisseur d'identité a refusé la connexion.
signin-error-state-mismatch = La demande de connexion a expiré ou a été ouverte dans un autre onglet.
signin-error-exchange-failed = Explorer n'a pas pu finaliser la connexion auprès du fournisseur d'identité.
signin-error-invalid-token = Le fournisseur d'identité a renvoyé une réponse invalide.
signin-error-missing-groups-claim = Votre compte ne comporte aucune information de groupe : impossible de vérifier vos droits d'accès. Demandez à un administrateur de vérifier la configuration du fournisseur d'identité.
signin-error-unknown = Une erreur est survenue pendant la connexion.
not-found-title = Page introuvable
not-found-message = Il n'y a rien à cette adresse.
no-volumes-title = Aucun volume
no-volumes-message = Explorer n'a encore aucun volume à afficher. Demandez à un administrateur d'ajouter un dossier dans le répertoire de données.
sidebar-open = Ouvrir la liste des volumes
sidebar-volumes = Volumes
sidebar-read-only = Lecture seule
sidebar-free = { $volume } · { $available } libres
sidebar-used-label = Espace utilisé
sidebar-used = { $used } utilisés sur { $total }
favorites-title = Favoris
favorites-not-found = Introuvable
favorites-remove = Retirer { $name } des favoris
favorites-load-failed = Les favoris n'ont pas pu être chargés.
favorites-add = Ajouter aux favoris
favorites-remove-current = Retirer des favoris
breadcrumb = Fil d'Ariane
browse-empty = Ce dossier est vide.
browse-empty-hint = Déposez des fichiers ici ou utilisez Uploader.
browse-empty-hint-touch = Utilisez + pour ajouter des fichiers.
read-only-notice = Lecture seule : vous pouvez parcourir, télécharger et copier depuis ce volume, mais pas le modifier.
read-only-notice-short = Volume en lecture seule
table-label = Contenu du dossier
table-name = Nom
table-size = Taille
table-modified = Modifié
table-actions = Actions
modified-yesterday = hier
download-file = Télécharger { $name }
download-folder = Télécharger { $name } en zip
entry-unavailable = Cet élément ne peut pas être ouvert.
error-reference = Référence : { $id }
error-open-volume = Ouvrir { $volume }
error-go-home = Revenir à l'accueil
error-unknown = Une erreur est survenue. Réessayez dans un instant.
error-volume_not_found = Le volume { $path } n'existe pas.
error-path_not_found = { $path } n'existe pas. Il a peut-être été déplacé ou supprimé.
error-invalid_path = Cette adresse ne désigne pas un emplacement valide.
error-not_a_directory = { $path } est un fichier, pas un dossier.
error-is_a_directory = { $path } est un dossier, pas un fichier.
error-permission_denied = Vous n'avez pas l'autorisation d'ouvrir { $path }.
error-outside_volume = { $path } pointe hors de son volume : impossible de l'ouvrir.
error-range_not_satisfiable = La partie demandée du fichier est hors limites.
error-internal = Une erreur est survenue sur le serveur. Réessayez dans un instant.
error-forbidden = Vous n'avez pas accès à cette ressource.
error-already_exists = { $path } existe déjà.
error-read_only_volume = Le volume { $path } est en lecture seule.
error-insufficient_storage = Il n'y a pas assez d'espace libre pour enregistrer { $path }.
error-upload_not_found = Cet upload n'existe plus sur le serveur.
error-offset_mismatch = L'upload s'est désynchronisé du serveur.
error-upload_busy = Cet upload est déjà en cours depuis un autre onglet ou appareil.
error-upload_too_large = Ce fichier est trop volumineux pour être uploadé.
error-invalid_upload = Le serveur a refusé cet upload.
error-unsupported_tus_version = Le serveur ne prend pas en charge cette version du protocole d'upload.
error-unsupported_media_type = Le serveur a refusé le format de cet upload.
error-cross_device = { $path } se trouve sur un autre système de fichiers que son volume, rien ne peut donc y être enregistré. Un administrateur peut monter ce dossier comme un volume à part entière.
error-invalid_destination = { $path } ne peut pas être déplacé ou copié à cet endroit.
error-task_not_found = Cette tâche n'existe plus sur le serveur.
error-favorites_full = Vous ne pouvez pas avoir plus de { $max } favoris. Retirez-en un d'abord.
error-thumbnail_unavailable = Aucun aperçu n'est disponible pour ce fichier.
error-too_many_tasks = Trop de tâches sont en attente. Réessayez quand certaines seront terminées.
upload = Uploader
upload-open = Uploader des fichiers ou un dossier
upload-files = Fichiers…
upload-folder = Dossier…
upload-read-only = Ce volume est en lecture seule : impossible d'y uploader quoi que ce soit.
uploads-cancel = Annuler { $name }
uploads-progress = Progression de l'upload
uploads-waiting = En attente
uploads-needs-decision = En attente de votre choix
uploads-creating-folder = Création du dossier…
uploads-sent = { $sent } / { $total }
uploads-resumable = reprise possible
uploads-retrying = Connexion perdue, nouvel essai dans { $seconds } s
uploads-done = Terminé
uploads-cancelled = Annulé
uploads-skipped = Ignoré
uploads-unreadable =
    { $count ->
        [one] 1 élément n'a pas pu être lu et a été ignoré.
       *[other] { $count } éléments n'ont pas pu être lus et ont été ignorés.
    }
uploads-retrying-server = Le serveur n'a pas pu le prendre, nouvel essai dans { $seconds } s
uploads-invalid-names =
    { $count ->
        [one] 1 élément a un nom inutilisable ici et a été ignoré.
       *[other] { $count } éléments ont un nom inutilisable ici et ont été ignorés.
    }
uploads-error-permission_denied = Vous n'avez pas l'autorisation d'enregistrer { $path }.
uploads-error-too-large = Le serveur ou un proxy devant lui a refusé cet upload, jugé trop volumineux.
conflict-title = { $name } existe déjà dans { $folder }
conflict-replace = Remplacer
conflict-keep-both = Garder les deux
conflict-apply-all = Appliquer à tous
upload-drop = Déposez pour uploader dans { $folder }
new-folder = Nouveau dossier
new-folder-title = Nouveau dossier
new-folder-name = Nom
new-folder-create = Créer
cancel = Annuler
name-error-empty = Saisissez un nom.
name-error-slash = Un nom ne peut pas contenir de barre oblique.
name-error-dot = Un nom ne peut pas être . ou ..
name-error-reserved = Ce nom est réservé.
name-error-too-long = Ce nom est trop long (255 octets maximum).
row-menu-open = Actions pour { $name }
action-download = Télécharger
action-rename = Renommer
action-move-to = Déplacer vers…
action-copy-to = Copier vers…
action-delete = Supprimer
select-all = Sélectionner toutes les entrées
select-entry = Sélectionner { $name }
selection-count =
    { $count ->
        [one] { $count } sélectionné
       *[other] { $count } sélectionnés
    }
selection-clear = Désélectionner
rename-title = Renommer { $name }
rename-name = Nouveau nom
rename-save = Enregistrer
activity-title =
    { $count ->
        [0] Activité
        [one] Activité · 1 en cours
       *[other] Activité · { $count } en cours
    }
activity-collapse = Réduire le panneau d'activité
activity-expand = Développer le panneau d'activité
activity-close = Fermer le panneau d'activité
task-deleting = Suppression de { $name }
task-copying = Copie de { $name } vers { $folder }
task-moving = Déplacement de { $name } vers { $folder }
task-cancel = Annuler { $name }
task-progress = Progression de la tâche
task-waiting = En attente
task-signed-out = Session terminée : reconnectez-vous pour suivre cette tâche.
task-cancel-failed = Cette tâche n'a pas pu être annulée.
task-files = { $done } / { $total }
task-files-done =
    { $count ->
        [one] 1 élément
       *[other] { $count } éléments
    }
task-done = Terminé
task-cancelled = Annulé
task-more =
    { $count ->
        [one] et 1 autre tâche
       *[other] et { $count } autres tâches
    }
delete-title =
    { $count ->
        [one] Supprimer { $name } ?
       *[other] Supprimer { $count } éléments ?
    }
delete-warning = Les dossiers sont supprimés avec tout leur contenu. Cette action est irréversible.
delete-confirm = Supprimer
task-deciding = Existe déjà : en attente de votre réponse
task-skipped =
    { $count ->
        [one] 1 élément n'a pas pu être copié et a été ignoré
       *[other] { $count } éléments n'ont pas pu être copiés et ont été ignorés
    }
task-skipped-move =
    { $count ->
        [one] 1 élément n'a pas pu être déplacé et est resté en place
       *[other] { $count } éléments n'ont pas pu être déplacés et sont restés en place
    }
copy-title =
    { $count ->
        [one] Copier { $name } vers…
       *[other] Copier { $count } éléments vers…
    }
move-title =
    { $count ->
        [one] Déplacer { $name } vers…
       *[other] Déplacer { $count } éléments vers…
    }
picker-volume = Volume
picker-choose-volume = Choisissez un volume
picker-volume-read-only = { $name } (lecture seule)
picker-location = Dossier de destination
picker-folders = Dossiers
picker-empty = Aucun dossier ici
picker-copy-here = Copier ici
picker-move-here = Déplacer ici
refresh = Actualiser
view-mode = Affichage
view-list = Affichage en liste
view-grid = Affichage en grille
grid-folder = Dossier
grid-count =
    { $count ->
        [one] { $count } élément
       *[other] { $count } éléments
    }
search-placeholder = Rechercher dans { $volume }
search-open = Rechercher
search-close = Fermer la recherche
search-searching = Recherche en cours…
search-results =
    { $count ->
        [one] 1 résultat pour « { $query } » dans { $volume }
       *[other] { $count } résultats pour « { $query } » dans { $volume }
    }
search-empty = Aucun résultat pour « { $query } » dans { $volume }
search-truncated = Affichage des { $count } premiers, affinez votre recherche.
search-timed-out = La recherche a pris trop de temps ; les résultats peuvent être incomplets.
action-analyze-usage = Analyser l’espace
action-calculate-size = Calculer la taille
analyze-usage = Analyser l’espace
folder-size-calculated = Calculée à { $time }
folder-size-computing = Calcul de la taille…
task-analysing = Analyse de { $name }
task-analysing-counts =
    { $count ->
        [one] Analyse de { $name } — { $files } fichier, { $size }
       *[other] Analyse de { $name } — { $files } fichiers, { $size }
    }
task-view = Voir
task-view-usage = Voir l’espace occupé par { $name }
usage-breadcrumb = Dossiers analysés
usage-items = Contenu
usage-empty = Ce dossier est vide.
usage-files =
    { $count ->
        [one] { $files } fichier
       *[other] { $files } fichiers
    }
usage-folders =
    { $count ->
        [one] { $folders } dossier
       *[other] { $folders } dossiers
    }
usage-other-files =
    { $count ->
        [one] { $files } autre fichier
       *[other] { $files } autres fichiers
    }
usage-percent = { $percent } %
usage-percent-small = < 1 %
usage-scanned-at = Analysé à { $time }
usage-scan-again = Analyser à nouveau
usage-open-in-explorer = Ouvrir dans l’explorateur
usage-incomplete = Certains éléments n’ont pas pu être lus
usage-collapsed = Trop de dossiers : les sous-dossiers de celui-ci sont comptés mais pas listés
usage-scanning = Analyse de { $name }…
usage-cancelled-title = Analyse annulée
usage-cancelled-message = Cette analyse a été arrêtée avant la fin.
usage-unavailable-title = Cette analyse n’est plus disponible
usage-unavailable-message = Les résultats ne sont conservés qu’un temps après l’analyse. Analysez à nouveau le dossier pour voir ce qui occupe son espace.
usage-failed-title = L’analyse a échoué
usage-scan-finished = Analyse terminée
