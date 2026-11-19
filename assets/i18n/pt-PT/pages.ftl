# login
login-restoring = A verificar a sua sessão guardada…
login-authorizing = A aguardar autorização no seu navegador…
login-signed-in = Sessão iniciada como { $name }
login-failed-title = Falha ao iniciar sessão
login-problem-region = O Spotify não abrirá uma sessão a partir do país onde está. Inicie sessão a partir do seu país de origem ou altere o país na sua conta do Spotify.
login-problem-credentials = A sua sessão guardada do Spotify já não é válida. Inicie sessão novamente para continuar.
login-problem-network = O Sonora não conseguiu aceder ao serviço de música. Verifique a sua ligação à internet e tente novamente.
login-problem-cancelled = Fechou a página do navegador antes de aprovar o início de sessão. Recomece para concluir.
login-problem-refused = O Spotify recusou o início de sessão. Aguarde um momento e tente novamente.
login-problem-premium = O Sonora usa o Spotify Premium para reproduzir, e esta conta não tem esse plano. Inicie sessão com uma conta Premium para continuar.
login-sign-in = Iniciar sessão com { $provider }
login-connect-cookies = Colar cookies manualmente
login-cookie-open = Abrir { $provider }
login-cookie-submit = Continuar
login-cookie-hint = Cole o cabeçalho Cookie aqui
login-cookie-step-1 = Aceda a music.youtube.com e certifique-se de que tem sessão iniciada. Funciona melhor numa janela anónima.
login-cookie-step-2 = Prima F12, abra o separador Rede e recarregue a página.
login-cookie-step-3 = Selecione qualquer pedido chamado "browse" ou "next".
login-cookie-step-4 = Em Cabeçalhos, encontre Cookie em Cabeçalhos do pedido, clique com o botão direito e copie o valor.
login-cookie-step-note = Cole o valor completo abaixo: o painel Cookies do pedido não é suficiente, pois o valor tem de conter SAPISID e __Secure-3PAPISID.
login-cookie-header-title = Cole os seus cookies do { $provider } para concluir o início de sessão
login-cookie-apple-step-3 = Selecione qualquer pedido para { $site }.
login-cookie-apple-note = Cole o valor completo, incluindo { $cookie }.
login-cookie-named-title = Cole o seu cookie do { $provider } para concluir o início de sessão
login-cookie-named-hint = Cole o valor do cookie aqui
login-cookie-named-step-1 = Aceda a { $site } e certifique-se de que tem sessão iniciada.
login-cookie-named-step-2 = Prima F12 e abra o separador Armazenamento no Firefox ou o separador Aplicação no Chrome.
login-cookie-named-step-3 = Expanda Cookies, selecione { $site } e encontre o cookie chamado { $cookie }.
login-cookie-named-step-4 = Faça duplo clique no valor e copie-o.
login-cookie-named-note = Só o valor de { $cookie } já basta. Um cabeçalho Cookie inteiro copiado do separador Rede também funciona.
login-guest-title = Modo convidado
login-guest-use = Usar modo convidado
login-usage-consent = Ajude-nos a estimar quantas pessoas usam o Sonora.
login-device-code = Introduza este código em { $url }
login-server-title = Ligue-se ao seu servidor Subsonic
login-server-detail = Introduza o endereço de qualquer servidor Subsonic ou OpenSubsonic (Navidrome, Airsonic, Gonic, …) e inicie sessão com o seu utilizador e palavra-passe do servidor. A sessão permanece neste dispositivo.
login-server-hint = https://music.example.com
login-username-hint = Utilizador
login-password-hint = Palavra-passe
login-server-submit = Ligar
login-account-title = Escolha uma conta
login-account-detail = Esta sessão está iniciada em mais de uma conta Google. Escolha qual o Sonora deve usar.
login-choose-title = Iniciar sessão no { $provider }
login-choose-detail = Escolha como iniciar sessão no { $provider }.

# album and playlist pages

# artist page
artist-eyebrow = Artista
artist-monthly-listeners = { $count ->
    [one] { $value } ouvinte mensal
   *[other] { $value } ouvintes mensais
}
artist-popular = Populares
artist-popular-eyebrow = Explorar este artista
artist-popular-empty = Nada para tocar deste artista ainda
artist-releases = Lançamentos
artist-filter-all = Tudo
artist-filter-singles = Singles
artist-filter-eps = EPs
artist-appears-on = Aparece em
album-also-like = Também pode gostar
album-tab-artists = Artistas
album-label = ℗ { $label }

# user profile page
user-eyebrow = Perfil
user-followers = { $count ->
    [one] { $value } seguidor
   *[other] { $value } seguidores
}
user-following = { $count ->
   *[other] { $value } a seguir
}
user-playlists = Playlists públicas
user-playlists-empty = Nenhuma playlist pública ainda

# home page
home-quick-picks = Escolhas rápidas
home-quick-picks-empty = Marque algumas músicas como favoritas e elas aparecerão aqui
home-recently-added = Adicionados recentemente
home-recently-played = Tocados recentemente
home-favorite-albums = Álbuns favoritos
home-artists = Artistas
home-collection-albums = Álbuns da sua coleção

# search page
search-placeholder = O que quer ouvir?
search-browse = Explorar tudo
search-submit = Pesquisar
search-recent = Pesquisas recentes
search-forget = Remover do histórico
genre-empty = Nada para mostrar aqui ainda
search-best-match = Melhor resultado
search-no-matches = Nenhum resultado
search-results = Resultados
search-artists = Artistas
search-albums-playlists = Álbuns e playlists
search-tag = { $kind } ·
search-saved =
    { $count ->
        [one] { $count } música na Biblioteca
       *[other] { $count } músicas na Biblioteca
    }

# song page
song-eyebrow = Música
song-view-album = Ver álbum
song-about = Sobre esta música
song-album = Álbum
song-released = Lançada
song-streams = Reproduções
song-position = Posição
song-label = Editora
song-popularity = Popularidade
song-disc-track = Disco { $disc }, faixa { $track }
song-track = Faixa { $track }
song-credits = Créditos
song-performed-by = Interpretada por
song-details = Géneros e detalhes
song-genres = Géneros
song-language = Idioma
song-content = Conteúdo
song-explicit = Explícito
song-clean = Limpo
artist-about = Sobre o artista
artist-about-fallback = Explore as músicas e lançamentos populares do artista.
artist-about-open = Ir para o artista
song-copyright = © { $notice }

# tags
tags-edit-title = Editar etiquetas
tags-sheet-song = Música
tags-sheet-album = Álbum
tags-sheet-details = Detalhes
tags-title = Título
tags-artist = Artista
tags-artists = Artistas
tags-add-artist = Adicionar um artista
tags-remove-artist = Remover este artista
tags-track = Número da faixa
tags-track-total = Faixas no lançamento
tags-disc = Número do disco
tags-disc-total = Discos no lançamento
tags-album = Álbum
tags-album-artist = Artista do álbum
tags-year = Ano
tags-genre = Género
tags-composer = Compositor
tags-publisher = Editora
tags-isrc = ISRC
tags-comment = Comentário
