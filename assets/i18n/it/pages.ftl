# login
login-restoring = Verifica della sessione salvata…
login-authorizing = In attesa dell'autorizzazione nel browser…
login-signed-in = Connesso come { $name }
login-failed-title = Accesso non riuscito
login-problem-region = Spotify non aprirà una sessione dal paese in cui ti trovi. Accedi dal tuo paese di residenza o cambia il paese sul tuo account Spotify.
login-problem-credentials = La tua sessione Spotify salvata non è più valida. Accedi di nuovo per continuare.
login-problem-network = Sonora non è riuscita a raggiungere il servizio musicale. Controlla la connessione internet e riprova.
login-problem-cancelled = Hai chiuso la pagina del browser prima di approvare l'accesso. Ricomincia per completare.
login-problem-refused = Spotify ha rifiutato l'accesso. Attendi un momento e riprova.
login-problem-premium = Sonora trasmette tramite Spotify Premium e questo account non lo possiede. Accedi con un account Premium per continuare.
login-sign-in = Accedi con { $provider }
login-connect-cookies = Incolla i cookie manualmente
login-cookie-open = Apri { $provider }
login-cookie-submit = Continua
login-cookie-hint = Incolla qui l'intestazione Cookie della richiesta
login-cookie-step-1 = Apri music.youtube.com e assicurati di aver effettuato l'accesso. Una finestra in incognito funziona meglio.
login-cookie-step-2 = Premi F12, apri la scheda Rete e ricarica la pagina.
login-cookie-step-3 = Seleziona una richiesta chiamata "browse" o "next".
login-cookie-step-4 = In Intestazioni, trova Cookie tra le intestazioni della richiesta, fai clic destro e copia il valore.
login-cookie-step-note = Incolla l'intero valore qui sotto: il pannello dei Cookie della richiesta non è sufficiente, perché il valore deve contenere SAPISID e __Secure-3PAPISID.
login-cookie-header-title = Incolla i cookie di { $provider } per completare l'accesso
login-cookie-apple-step-3 = Seleziona una richiesta qualsiasi a { $site }.
login-cookie-apple-note = Assicurati di incollare l'intero valore, incluso { $cookie }.
login-cookie-named-title = Incolla il cookie di { $provider } per completare l'accesso
login-cookie-named-hint = Incolla qui il valore del cookie
login-cookie-named-step-1 = Apri { $site } e assicurati di aver effettuato l'accesso.
login-cookie-named-step-2 = Premi F12 e apri la scheda Archiviazione in Firefox, o la scheda Applicazione in Chrome.
login-cookie-named-step-3 = Espandi Cookie, seleziona { $site } e trova il cookie chiamato { $cookie }.
login-cookie-named-step-4 = Fai doppio clic sul suo valore e copialo.
login-cookie-named-note = Basta il solo valore di { $cookie }. Funziona anche un'intera intestazione Cookie copiata dalla scheda Rete.
login-guest-title = Modalità ospite
login-guest-use = Usa modalità ospite
login-usage-consent = Aiutaci a stimare quante persone usano Sonora.
login-device-code = Inserisci questo codice su { $url }
login-server-title = Connettiti al tuo server Subsonic
login-server-detail = Inserisci l'indirizzo di un server Subsonic o OpenSubsonic (Navidrome, Airsonic, Gonic, …), poi accedi con nome utente e password del server. La sessione rimane su questo dispositivo.
login-server-hint = https://music.example.com
login-username-hint = Nome utente
login-password-hint = Password
login-server-submit = Connetti
login-account-title = Scegli un account
login-account-detail = Questa sessione ha effettuato l'accesso a più di un account Google. Scegli quello che Sonora deve usare.
login-choose-title = Accedi a { $provider }
login-choose-detail = Scegli come accedere a { $provider }.

# album and playlist pages

# artist page
artist-eyebrow = Artista
artist-monthly-listeners = { $count ->
    [one] { $value } ascoltatore al mese
   *[other] { $value } ascoltatori al mese
}
artist-popular = Popolari
artist-popular-eyebrow = Esplora questo artista
artist-popular-empty = Ancora nulla da riprodurre di questo artista
artist-releases = Pubblicazioni
artist-filter-all = Tutti
artist-filter-singles = Singoli
artist-filter-eps = EP
artist-appears-on = Appare in
album-also-like = Potrebbe piacerti anche
album-tab-artists = Artisti
album-label = ℗ { $label }

# user profile page
user-eyebrow = Profilo
user-followers = { $count ->
    [one] { $value } follower
   *[other] { $value } follower
}
user-following = { $count ->
    [one] { $value } seguito
   *[other] { $value } seguiti
}
user-playlists = Playlist pubbliche
user-playlists-empty = Ancora nessuna playlist pubblica

# home page
home-quick-picks = Scelte rapide
home-quick-picks-empty = Metti Mi piace a qualche brano e apparirà qui
home-recently-added = Aggiunti di recente
home-recently-played = Ascoltati di recente
home-favorite-albums = Album preferiti
home-artists = Artisti
home-collection-albums = Album dalla tua collezione

# search page
search-placeholder = Cosa vuoi ascoltare?
search-browse = Sfoglia tutto
search-submit = Cerca
search-recent = Ricerche recenti
search-forget = Rimuovi dalla cronologia
genre-empty = Nulla da mostrare qui
search-best-match = Miglior risultato
search-no-matches = Nessun risultato
search-results = Risultati
search-artists = Artisti
search-albums-playlists = Album e playlist
search-tag = { $kind } ·
search-saved = { $count ->
    [one] { $count } brano in libreria
   *[other] { $count } brani in libreria
}

# song page
song-eyebrow = Brano
song-view-album = Mostra album
song-about = Informazioni sul brano
song-album = Album
song-released = Pubblicato
song-streams = Stream
song-position = Posizione
song-label = Etichetta
song-popularity = Popolarità
song-disc-track = Disco { $disc }, traccia { $track }
song-track = Traccia { $track }
song-credits = Crediti
song-performed-by = Interpretato da
song-details = Generi e dettagli
song-genres = Generi
song-language = Lingua
song-content = Contenuto
song-explicit = Esplicito
song-clean = Pulito
artist-about = Sull'artista
artist-about-fallback = Esplora i brani popolari e le pubblicazioni dell'artista.
artist-about-open = Vai all'artista
song-copyright = © { $notice }

# tags
tags-edit-title = Modifica tag
tags-sheet-song = Brano
tags-sheet-album = Album
tags-sheet-details = Dettagli
tags-title = Titolo
tags-artist = Artista
tags-artists = Artisti
tags-add-artist = Aggiungi un artista
tags-remove-artist = Rimuovi questo artista
tags-track = Numero traccia
tags-track-total = Tracce nella release
tags-disc = Numero disco
tags-disc-total = Dischi nella release
tags-album = Album
tags-album-artist = Artista album
tags-year = Anno
tags-genre = Genere
tags-composer = Compositore
tags-publisher = Editore
tags-isrc = ISRC
tags-comment = Commento
