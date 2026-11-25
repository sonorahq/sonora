# common
common-left = Gauche
common-right = Droite
common-search = Rechercher
common-unknown = Inconnu
common-not-provided = Non renseigné
common-not-available = Indisponible
common-unavailable = { $name } (indisponible)
common-cancel = Annuler
common-save = Enregistrer
common-delete = Supprimer
common-play = Lire
common-more = Plus
common-previous = Précédent
common-next = Suivant
common-dismiss = Fermer
common-clear = Effacer
common-undo = Annuler
common-redo = Rétablir
common-cut = Couper
common-copy = Copier
common-paste = Coller
common-show-all = Tout afficher
common-show-less = Afficher moins
common-local-files = Fichiers locaux
common-server-address = Adresse du serveur
common-api-key = Clé API
common-not-now = Plus tard
common-not-connected = Non connecté
common-sonora = Sonora
common-more-count = +{ $count }
number-group = { " " }

# units
unit-percent = { $value } %
unit-db = { $value } dB
unit-hz = { $value } Hz
unit-khz = { $value } kHz
unit-kbps = { $value } kbit/s
unit-gb = { $value } Go
unit-mb = { $value } Mo
unit-fps = { $value } FPS
unit-px = { $value } px
runtime-seconds = { $seconds } s
runtime-minutes = { $minutes } min { $seconds } s
runtime-hours = { $hours } h { $minutes } min

# navigation
nav-history = Historique
nav-home = Accueil
nav-search = Recherche
nav-library = Ma bibliothèque
nav-settings = Paramètres
nav-songs = Titres
nav-albums = Albums
nav-playlists = Playlists
nav-artists = Artistes
nav-local = Musique locale
nav-back = Retour
nav-forward = Suivant
nav-sidebar = Afficher ou masquer la barre latérale
nav-sidebar-right = Afficher ou masquer les paroles et la file d'attente
nav-pinned = Épinglés
nav-unpin = Détacher
nav-pin-hint = Déposer ici pour épingler
library-liked-songs = Favoris
library-no-songs = Aucun favori pour l’instant
library-no-albums = Aucun album enregistré pour l'instant
library-no-playlists = Aucune playlist pour l'instant
library-no-artists = Aucun artiste favori pour l'instant
library-no-local-songs = Aucun titre importé trouvé
library-no-local-albums = Aucun album importé trouvé
library-no-local-artists = Aucun artiste importé
library-no-local-playlists = Aucune playlist locale
library-no-catalog-songs = Aucun titre trouvé
library-no-catalog-albums = Aucun album trouvé
library-no-catalog-artists = Aucun artiste trouvé
library-no-matches = Aucun résultat
library-not-loaded = Votre bibliothèque ne s’est pas chargée
library-scanning = Analyse de vos dossiers…
library-scanning-progress = Lecture de votre musique… { $read } sur { $found }
library-part-not-loaded = Cette partie de votre bibliothèque ne s’est pas chargée
library-local-unconfigured = Configurez votre bibliothèque locale

# app menu
app-refresh = Actualiser
app-sign-out = Se déconnecter
app-quit = Quitter
app-settings = Réglages…
app-hide = Masquer Sonora
app-hide-others = Masquer les autres
app-show-all = Tout afficher
app-edit = Édition
app-select-all = Tout sélectionner
app-window = Fenêtre
app-close-window = Fermer la fenêtre
app-minimize = Placer dans le Dock
app-zoom = Réduire/agrandir

# tray menu
tray-show = Afficher Sonora
tray-play = Lire
tray-pause = Pause

# table columns
column-played-at = Écouté
column-index = #
column-title = Titre
column-artist = Artiste
column-album = Album
column-genre = Genre
column-date-added = Date d'ajout
column-added-by = Ajouté par
column-modified = Modifié
column-length = Durée
column-plays = Écoutes
column-name = Nom
column-owner = Propriétaire
column-year = Année
column-tracks = Titres

# filters
filter-history = Filtrer l’historique d’écoute
history-empty = Les titres écoutés apparaîtront ici.
history-not-loaded = Impossible de charger l’historique d’écoute.
history-clear = Effacer l’historique
history-clear-title = Effacer l’historique d’écoute
history-clear-confirm = Toutes les écoutes seront supprimées de cet appareil. Cette action est irréversible.
filter-library = Filtrer la bibliothèque
filter-album = Filtrer les titres de l'album
filter-reset = Réinitialiser les filtres
filter-duration = Durée
filter-year = Année
filter-hide-explicit = Masquer les explicites
filter-favorites = Favoris uniquement
filter-downloaded = Téléchargés
filter-owned = Par vous

# view
view-list = Liste
view-cards = Cartes

# toolbar
tool-sort = Trier
tool-filters = Filtres

# release kinds
release-album = Album
release-single = Single
release-compilation = Compilation
release-ep = EP
release-audiobook = Livre audio
release-podcast = Podcast
kind-song = Titre
kind-artist = Artiste
kind-album = Album
kind-playlist = Playlist

# song languages
language-ar = Arabe
language-de = Allemand
language-en = Anglais
language-es = Espagnol
language-fr = Français
language-hi = Hindi
language-it = Italien
language-ja = Japonais
language-ko = Coréen
language-pt = Portugais
language-ru = Russe
language-tr = Turc
language-uk = Ukrainien
language-zh = Chinois
language-zxx = Aucun contenu linguistique

# counts
count-songs =
    { $count ->
        [one] { $count } titre
       *[other] { $count } titres
    }
count-tracks =
    { $count ->
        [one] { $count } titre
       *[other] { $count } titres
    }

# dates
date-just-now = À l'instant
date-minute-ago = Il y a une minute
date-minutes-ago = Il y a { $count } minutes
date-today = Aujourd'hui à { $time }
date-yesterday = Hier à { $time }
date-time = { $date }, { $time }
date-full = { $day } { $month } { $year }
month-1 = janv.
month-2 = févr.
month-3 = mars
month-4 = avr.
month-5 = mai
month-6 = juin
month-7 = juil.
month-8 = août
month-9 = sept.
month-10 = oct.
month-11 = nov.
month-12 = déc.

nav-pin = Épingler
nav-nothing-pinned = Rien ici
nav-pins-alphabetical = Alphabétique
nav-pins-kind = Par type
nav-show-full-library = Afficher toute la bibliothèque
nav-return-top = Revenir en haut

# power
wake-reason = Musique en cours de lecture
