# track menu
menu-new-playlist = Nouvelle playlist
menu-import-playlist = Importer une playlist…
menu-no-playlists = Aucune playlist
menu-search-playlists = Rechercher une playlist
menu-add-to-library = Ajouter aux favoris
menu-remove-from-library = Retirer des favoris
menu-remove-tracks-from-playlist = { $count ->
    [one] Retirer { $count } titre de la playlist
   *[other] Retirer { $count } titres de la playlist
}
menu-remove-tracks-from-history = { $count ->
    [one] Retirer { $count } titre de l'historique
   *[other] Retirer { $count } titres de l'historique
}
menu-download = Télécharger
menu-remove-download = Supprimer le téléchargement
menu-downloading = Téléchargement en cours. Cliquer pour arrêter
menu-song-radio = Ouvrir la radio du titre
menu-go-to-album = Aller à l'album
menu-go-to-artist = Aller à l'artiste
menu-view-details = Voir les détails
menu-copy-link = Copier le lien
menu-select-all = Tout sélectionner
menu-remove-from-queue = Retirer de la file d'attente
menu-export-playlist = Exporter la playlist…
menu-add-playlist-to-library = Ajouter à la bibliothèque
menu-remove-playlist-from-library = Retirer de la bibliothèque
menu-make-playlist-public = Rendre publique
menu-make-playlist-private = Rendre privée
menu-play-artist = Lire l'artiste

# playlist editor
menu-copy-link-none = Aucun lien à partager
menu-copy-link-local = Les artistes et playlists locaux n'ont pas de lien
menu-play-next = { $count ->
    [1] Lire juste après
    [one] Lire { $count } titre juste après
   *[other] Lire { $count } titres juste après
}
menu-add-to-queue = { $count ->
    [1] Ajouter à la file d'attente
    [one] Ajouter { $count } titre à la file d'attente
   *[other] Ajouter { $count } titres à la file d'attente
}
menu-play-last = { $count ->
    [1] Lire en dernier
    [one] Lire { $count } titre en dernier
   *[other] Lire { $count } titres en dernier
}
menu-delete-track-files = { $count ->
    [1] Supprimer le fichier du titre
    [one] Supprimer le fichier de { $count } titre
   *[other] Supprimer les fichiers de { $count } titres
}

# playlist editor
playlist-name-placeholder = Nom de la playlist
playlist-create-title = Créer une playlist
playlist-edit-title = Modifier la playlist
playlist-cover-choose = Choisir une image
playlist-cover-remove = Retirer la couverture
playlist-cover-unopenable = Sonora ne peut pas ouvrir ce fichier
playlist-cover-too-large = Cette image dépasse 40 Mo
playlist-cover-not-a-picture = Ce fichier n'est pas une image JPEG, PNG ou WebP
playlist-delete-title = Supprimer la playlist
playlist-again-title = L'ajouter encore ?
playlist-again-confirm = Ce titre est déjà dans « { $name } ». En ajouter une copie ?
playlist-again-add = Ajouter quand même

# confirm

# confirm
confirm-remove-library-title = Retirer de la bibliothèque
confirm-remove-playlist-title = Retirer de la playlist
confirm-remove-history-title = Retirer de l'historique
confirm-remove-songs = { $count ->
    [one] Retirer ce titre de votre bibliothèque ?
   *[other] Retirer { $count } titres de votre bibliothèque ?
}
confirm-remove-playlist-songs = { $count ->
    [one] Retirer ce titre de la playlist ?
   *[other] Retirer { $count } titres de la playlist ?
}
confirm-remove-history-songs = { $count ->
    [one] Retirer ce titre de l'historique d'écoute ?
   *[other] Retirer { $count } titres de l'historique d'écoute ?
}
confirm-remove-albums = { $count ->
    [one] Retirer cet album de votre bibliothèque ?
   *[other] Retirer { $count } albums de votre bibliothèque ?
}
confirm-remove-artists = { $count ->
    [one] Retirer cet artiste des favoris ?
   *[other] Retirer { $count } artistes des favoris ?
}
confirm-remove-playlists = { $count ->
    [one] Retirer cette playlist de votre bibliothèque ?
   *[other] Retirer { $count } playlists de votre bibliothèque ?
}
confirm-delete-track-files-title = Supprimer les fichiers des titres ?
confirm-delete-track-files = Le ou les fichiers seront définitivement supprimés du disque. Cette action est irréversible.
confirm-uninstall-widevine-title = Désinstaller le module Widevine ?
confirm-uninstall-widevine = Sonora supprime la copie téléchargée depuis Google. Les titres Apple Music ne pourront plus être lus tant que Sonora ne l'aura pas téléchargée à nouveau.
confirm-remove-downloads-title = Supprimer tous les téléchargements ?
confirm-remove-downloads = Tous les titres téléchargés sont supprimés de cet ordinateur, pour tous les services. Ils seront de nouveau lus en streaming jusqu'à ce que vous les téléchargiez à nouveau.
confirm-delete-named = Supprimer « { $name } » ? Cette action est irréversible.
