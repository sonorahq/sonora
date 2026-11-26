# track menu
menu-new-playlist = Nuova playlist
menu-import-playlist = Importa playlist…
menu-no-playlists = Nessuna playlist
menu-search-playlists = Cerca una playlist
menu-save-library = Libreria
menu-add-to-library = Aggiungi ai Preferiti
menu-remove-from-library = Rimuovi dai Preferiti
menu-remove-tracks-from-playlist = { $count ->
    [one] Rimuovi { $count } brano dalla playlist
   *[other] Rimuovi { $count } brani dalla playlist
}
menu-remove-tracks-from-history = { $count ->
    [one] Rimuovi { $count } brano dalla cronologia
   *[other] Rimuovi { $count } brani dalla cronologia
}
menu-download = Scarica
menu-remove-download = Rimuovi download
menu-downloading = Download in corso. Fai clic per interrompere
menu-start-radio = Avvia radio
menu-go-to-album = Vai all'album
menu-go-to-artist = Vai all'artista
menu-go-to-playlist = Vai alla playlist
menu-view-details = Mostra dettagli
menu-copy-link = Copia link
menu-select-all = Seleziona tutto
menu-remove-from-queue = Rimuovi dalla coda
menu-export-playlist = Esporta playlist…
menu-add-playlist-to-library = Aggiungi alla Libreria
menu-remove-playlist-from-library = Rimuovi dalla Libreria
menu-make-playlist-public = Rendi pubblica
menu-make-playlist-private = Rendi privata
menu-play-artist = Riproduci artista

# playlist editor
menu-copy-link-none = Non c'è un link da condividere
menu-copy-link-local = Artisti e playlist locali non hanno un link
menu-play-next = { $count ->
    [1] Riproduci per prossimo
    [one] Riproduci { $count } brano per prossimo
   *[other] Riproduci { $count } brani per prossimi
}
menu-add-to-queue = { $count ->
    [1] Aggiungi alla coda
    [one] Aggiungi { $count } brano alla coda
   *[other] Aggiungi { $count } brani alla coda
}
menu-play-last = { $count ->
    [1] Riproduci per ultimo
    [one] Riproduci { $count } brano per ultimo
   *[other] Riproduci { $count } brani per ultimi
}
menu-delete-track-files = { $count ->
    [1] Elimina file del brano
   *[other] Elimina { $count } file dei brani
}
menu-add-to-playlist = { $count ->
    [1] Aggiungi alla playlist
    [one] Aggiungi { $count } brano alla playlist
   *[other] Aggiungi { $count } brani alla playlist
}

# smart rules
smart-rules-create-title = Nuova playlist smart
smart-rules-edit-title = Regole smart
smart-rules-loading = Lettura delle regole dal server…
smart-rules-saving = Salvataggio…
smart-rules-failed-title = Impossibile leggere le regole
smart-rules-match-all = Soddisfa tutte
smart-rules-match-any = Soddisfa almeno una
smart-rules-match-toggle = Soddisfa tutte queste regole o almeno una di esse
smart-rules-of-following = delle seguenti
smart-rules-add-condition = Aggiungi condizione
smart-rules-add-group = Aggiungi gruppo
smart-rules-remove-group = Rimuovi questo gruppo
smart-rules-remove-condition = Rimuovi questa regola
smart-rules-empty-group = Ancora nessuna regola qui
smart-rules-field = Campo
smart-rules-operator = Operatore
smart-rules-value-placeholder = Valore
smart-rules-range-and = e
smart-rules-true = Sì
smart-rules-false = No
smart-rules-unknown-note = Mantenuto come inviato dal server
smart-rules-synced-title = Questa playlist si sincronizza da un file
smart-rules-synced-message = La prossima scansione può sovrascrivere le modifiche fatte qui.
smart-rules-sort-label = Ordina per
smart-rules-sort-placeholder = title, dateloved, random…
smart-rules-order-none = Nessuna direzione
smart-rules-order-asc = Crescente
smart-rules-order-desc = Decrescente
smart-rules-order-toggle = Direzione di ordinamento
smart-rules-limit-label = Limite
smart-rules-limit-placeholder = Tutti i risultati
smart-rules-problem-empty-group = Il gruppo non contiene regole
smart-rules-problem-missing-field = La condizione non indica alcun campo
smart-rules-problem-range = Un intervallo richiede un limite inferiore e uno superiore
smart-rules-problem-days = Un conteggio di giorni richiede un numero
smart-rules-problem-playlist = Una regola playlist richiede una playlist
smart-rules-problem-truth = La regola richiede un valore di verità
smart-rules-problem-compared = Il valore non è adatto alla regola
smart-rules-problem-boolean = Un campo booleano richiede un valore di verità
smart-rules-problem-bad-order = L'ordine è asc o desc

# playlist editor
playlist-name-placeholder = Nome playlist
playlist-create-title = Crea playlist
playlist-create-shelf = Salva in
playlist-edit-title = Modifica playlist
playlist-cover-choose = Scegli immagine
playlist-cover-remove = Rimuovi copertina
playlist-cover-unopenable = Sonora non riesce ad aprire questo file
playlist-cover-too-large = Quest'immagine supera i 40 MB
playlist-cover-not-a-picture = Questo file non è un'immagine JPEG, PNG o WebP

# cover viewer
playlist-delete-title = Elimina playlist
playlist-again-title = Aggiungere di nuovo?
playlist-again-confirm = Questo brano è già in “{ $name }”. Aggiungere un'altra copia?
playlist-again-add = Aggiungi di nuovo

# confirm

# cover viewer
cover-view = Mostra copertina
cover-zoom-in = Ingrandisci
cover-zoom-out = Riduci
cover-zoom-fit = Adatta alla finestra

# confirm
confirm-delete-track-files-title = Eliminare i file dei brani?
confirm-delete-track-files = I file verranno eliminati definitivamente dal disco. Questa operazione non può essere annullata.
confirm-uninstall-widevine-title = Disinstallare il modulo Widevine?
confirm-uninstall-widevine = Sonora elimina la copia scaricata da Google. I brani di Apple Music non verranno riprodotti finché non lo scaricherai di nuovo.
confirm-remove-soloist-title = Rimuovere Soloist?
confirm-remove-soloist = Sonora elimina il programma Soloist, la tua chiave API e il suo accesso. Spotify tornerà al lettore integrato.
confirm-remove-downloads-title = Rimuovere tutti i download?
confirm-remove-downloads = Ogni brano scaricato verrà eliminato da questo computer, per tutti i servizi. Torneranno in streaming finché non li scaricherai di nuovo.
confirm-delete-named = Eliminare “{ $name }”? Questa operazione non può essere annullata.