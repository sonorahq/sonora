# track menu
menu-new-playlist = Neue Playlist
menu-import-playlist = Playlist importieren…
menu-no-playlists = Keine Playlists
menu-search-playlists = Playlist suchen
menu-add-to-library = Zu Favoriten hinzufügen
menu-remove-from-library = Aus Favoriten entfernen
menu-remove-tracks-from-playlist = { $count ->
    [one] { $count } Track aus Playlist entfernen
   *[other] { $count } Tracks aus Playlist entfernen
}
menu-remove-tracks-from-history = { $count ->
    [one] { $count } Track aus Verlauf entfernen
   *[other] { $count } Tracks aus Verlauf entfernen
}
menu-download = Herunterladen
menu-remove-download = Download entfernen
menu-downloading = Wird heruntergeladen. Zum Stoppen klicken
menu-song-radio = Song-Radio öffnen
menu-go-to-album = Zum Album
menu-go-to-artist = Zum Künstler
menu-view-details = Details anzeigen
menu-copy-link = Link kopieren
menu-select-all = Alle auswählen
menu-remove-from-queue = Aus der Warteschlange entfernen
menu-export-playlist = Playlist exportieren…
menu-add-playlist-to-library = Zur Bibliothek hinzufügen
menu-remove-playlist-from-library = Aus der Bibliothek entfernen
menu-make-playlist-public = Öffentlich machen
menu-make-playlist-private = Privat machen
menu-play-artist = Künstler spielen

# playlist editor
menu-copy-link-none = Hierfür gibt es keinen Link zum Teilen
menu-copy-link-local = Lokale Künstler und Playlists haben keinen Link
menu-play-next = { $count ->
    [1] Als Nächstes spielen
    [one] { $count } Track als Nächstes spielen
   *[other] { $count } Tracks als Nächstes spielen
}
menu-add-to-queue = { $count ->
    [1] Zur Warteschlange hinzufügen
    [one] { $count } Track zur Warteschlange hinzufügen
   *[other] { $count } Tracks zur Warteschlange hinzufügen
}
menu-play-last = { $count ->
    [1] Als Letztes spielen
    [one] { $count } Track als Letztes spielen
   *[other] { $count } Tracks als Letztes spielen
}
menu-delete-track-files = { $count ->
    [1] Titeldatei löschen
   *[other] { $count } Titeldateien löschen
}

# playlist editor
playlist-name-placeholder = Name der Playlist
playlist-create-title = Playlist erstellen
playlist-edit-title = Playlist bearbeiten
playlist-cover-choose = Bild auswählen
playlist-cover-remove = Cover entfernen
playlist-cover-unopenable = Sonora kann diese Datei nicht öffnen
playlist-cover-too-large = Dieses Bild ist größer als 40 MB
playlist-cover-not-a-picture = Diese Datei ist kein JPEG-, PNG- oder WebP-Bild
playlist-delete-title = Playlist löschen
playlist-again-title = Noch einmal hinzufügen?
playlist-again-confirm = Dieser Titel ist schon in „{ $name }“. Noch ein Mal hinzufügen?
playlist-again-add = Erneut hinzufügen

# confirm

# confirm
confirm-remove-library-title = Von Bibliothek entfernen
confirm-remove-playlist-title = Von Playlist entfernen
confirm-remove-history-title = Von Verlauf entfernen
confirm-remove-songs = { $count ->
    [one] Den Song von der Bibliothek entfernen?
   *[other] { $count } Songs von der Bibliothek entfernen?
}
confirm-remove-playlist-songs = { $count ->
    [one] Den Song von der Playlist entfernen?
   *[other] { $count } Songs von der Playlist entfernen?
}
confirm-remove-history-songs = { $count ->
    [one] Den Song von dem Verlauf entfernen?
   *[other] { $count } Songs von dem Verlauf entfernen?
}
confirm-remove-albums = { $count ->
    [one] Den Album von der Bibliothek entfernen?
   *[other] { $count } Alben von der Bibliothek entfernen?
}
confirm-remove-artists = { $count ->
    [one] Diesen Künstler aus den Favoriten entfernen?
   *[other] { $count } Künstler aus den Favoriten entfernen?
}
confirm-remove-playlists = { $count ->
    [one] Den Playlist von der Bibliothek entfernen?
   *[other] { $count } Playlists von der Bibliothek entfernen?
}
confirm-delete-track-files-title = Titeldateien löschen?
confirm-delete-track-files = Die Datei(en) werden endgültig von der Festplatte gelöscht. Das lässt sich nicht rückgängig machen.
confirm-uninstall-widevine-title = Widevine-Modul deinstallieren?
confirm-uninstall-widevine = Sonora löscht die Kopie, die es von Google geladen hat. Apple-Music-Titel spielen erst wieder, wenn es erneut heruntergeladen ist.
confirm-remove-downloads-title = Alle Downloads entfernen?
confirm-remove-downloads = Alle heruntergeladenen Songs aller Dienste werden von diesem Computer gelöscht. Sie werden wieder gestreamt, bis du sie erneut herunterlädst.
confirm-delete-named = „{ $name }“ löschen? Das lässt sich nicht rückgängig machen.
