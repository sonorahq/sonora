# login
login-restoring = Vérification de votre session enregistrée…
login-authorizing = En attente de l'autorisation dans votre navigateur…
login-signed-in = Connecté en tant que { $name }
login-failed-title = Échec de la connexion
login-problem-region = Spotify n'ouvre pas de session depuis le pays où vous êtes. Connectez-vous depuis votre pays d'origine, ou changez le pays de votre compte Spotify.
login-problem-credentials = Votre session Spotify enregistrée n'est plus valide. Reconnectez-vous pour continuer.
login-problem-network = Sonora n'a pas pu joindre le service de musique. Vérifiez votre connexion internet et réessayez.
login-problem-cancelled = Vous avez fermé la page du navigateur avant d'approuver la connexion. Recommencez pour aller au bout.
login-problem-refused = Spotify a refusé la connexion. Patientez un instant et réessayez.
login-problem-premium = Sonora diffuse via Spotify Premium, et ce compte ne l'a pas. Connectez-vous avec un compte Premium pour continuer.
login-sign-in = Se connecter avec { $provider }
login-connect-cookies = Coller les cookies manuellement
login-cookie-open = Ouvrir { $provider }
login-cookie-submit = Continuer
login-cookie-hint = Collez ici l'en-tête de requête Cookie
login-cookie-step-1 = Ouvrez music.youtube.com et vérifiez que vous êtes connecté. Une fenêtre de navigation privée fonctionne le mieux.
login-cookie-step-2 = Appuyez sur F12, ouvrez l'onglet Réseau et rechargez la page.
login-cookie-step-3 = Sélectionnez une requête nommée « browse » ou « next ».
login-cookie-step-4 = Dans les en-têtes, trouvez Cookie sous les en-têtes de requête, faites un clic droit dessus et copiez sa valeur.
login-cookie-step-note = Collez la valeur entière ci-dessous : le panneau Cookies de la requête ne suffit pas, car la valeur doit contenir SAPISID et __Secure-3PAPISID.
login-cookie-header-title = Collez vos cookies { $provider } pour terminer la connexion
login-cookie-apple-step-3 = Sélectionnez n'importe quelle requête vers { $site }.
login-cookie-apple-note = Veillez à coller la valeur entière, y compris { $cookie }.
login-cookie-named-title = Collez votre cookie { $provider } pour terminer la connexion
login-cookie-named-hint = Collez ici la valeur du cookie
login-cookie-named-step-1 = Ouvrez { $site } et vérifiez que vous êtes connecté.
login-cookie-named-step-2 = Appuyez sur F12 et ouvrez l'onglet Stockage dans Firefox, ou l'onglet Application dans Chrome.
login-cookie-named-step-3 = Dépliez Cookies, sélectionnez { $site } et trouvez le cookie nommé { $cookie }.
login-cookie-named-step-4 = Double-cliquez sur sa valeur et copiez-la.
login-cookie-named-note = La valeur de { $cookie } seule suffit. Un en-tête Cookie entier copié depuis l'onglet Réseau fonctionne aussi.
login-guest-title = Mode invité
login-guest-use = Utiliser le mode invité
login-usage-consent = Aidez-nous à estimer combien de personnes utilisent Sonora.
login-device-code = Saisissez ce code sur { $url }
login-server-title = Connectez-vous à votre serveur Subsonic
login-server-detail = Saisissez l'adresse d'un serveur Subsonic ou OpenSubsonic (Navidrome, Airsonic, Gonic, …), puis connectez-vous avec votre nom d'utilisateur et votre mot de passe du serveur. La session reste sur cet appareil.
login-server-hint = https://music.example.com
login-username-hint = Nom d'utilisateur
login-password-hint = Mot de passe
login-server-submit = Se connecter
login-account-title = Choisir un compte
login-account-detail = Cette session est connectée à plusieurs comptes Google. Choisissez celui que Sonora doit utiliser.
login-choose-title = Se connecter à { $provider }
login-choose-detail = Choisissez comment vous connecter à { $provider }.

# album and playlist pages

# artist page
artist-eyebrow = Artiste
artist-monthly-listeners = { $count ->
    [one] { $value } auditeur mensuel
   *[other] { $value } auditeurs mensuels
}
artist-popular = Populaires
artist-popular-eyebrow = Découvrir cet artiste
artist-popular-empty = Rien à écouter de cet artiste pour l'instant
artist-releases = Sorties
artist-filter-all = Toutes
artist-filter-singles = Singles
artist-filter-eps = EP
artist-appears-on = Apparaît sur
album-also-like = Vous aimerez peut-être aussi
album-tab-artists = Artistes
album-label = ℗ { $label }

# user profile page
user-eyebrow = Profil
user-followers = { $count ->
    [one] { $value } abonné
   *[other] { $value } abonnés
}
user-following = { $count ->
    [one] { $value } abonnement
   *[other] { $value } abonnements
}
user-playlists = Playlists publiques
user-playlists-empty = Aucune playlist publique pour le moment

# home page
home-quick-picks = Sélection rapide
home-quick-picks-empty = Likez quelques titres et ils apparaîtront ici
home-recently-added = Ajoutés récemment
home-recently-played = Écoutés récemment
home-favorite-albums = Albums favoris
home-artists = Artistes
home-collection-albums = Albums de votre collection

# search page
search-placeholder = Que voulez-vous écouter ?
search-browse = Tout parcourir
search-submit = Rechercher
search-recent = Recherches récentes
search-forget = Retirer de l'historique
genre-empty = Rien à afficher ici pour l'instant
search-best-match = Meilleur résultat
search-no-matches = Aucun résultat
search-results = Résultats
search-artists = Artistes
search-albums-playlists = Albums et playlists
search-tag = { $kind } ·
search-saved =
    { $count ->
        [one] { $count } titre dans la bibliothèque
       *[other] { $count } titres dans la bibliothèque
    }

# song page
song-eyebrow = Titre
song-view-album = Voir l'album
song-about = À propos de ce titre
song-album = Album
song-released = Sortie
song-streams = Écoutes
song-position = Position
song-label = Label
song-popularity = Popularité
song-disc-track = Disque { $disc }, piste { $track }
song-track = Piste { $track }
song-credits = Crédits
song-performed-by = Interprété par
song-details = Genres et détails
song-genres = Genres
song-language = Langue
song-content = Contenu
song-explicit = Explicite
song-clean = Tout public
artist-about = À propos de l'artiste
artist-about-fallback = Découvrez les titres populaires et les sorties de l'artiste.
artist-about-open = Aller à l'artiste
song-copyright = © { $notice }

# tags
tags-edit-title = Modifier les tags
tags-sheet-song = Titre
tags-sheet-album = Album
tags-sheet-details = Détails
tags-title = Titre
tags-artist = Artiste
tags-artists = Artistes
tags-add-artist = Ajouter un artiste
tags-remove-artist = Retirer cet artiste
tags-track = Numéro de piste
tags-track-total = Titres dans la sortie
tags-disc = Numéro de disque
tags-disc-total = Disques dans la sortie
tags-album = Album
tags-album-artist = Artiste de l’album
tags-year = Année
tags-genre = Genre
tags-composer = Compositeur
tags-publisher = Éditeur
tags-isrc = ISRC
tags-comment = Commentaire
