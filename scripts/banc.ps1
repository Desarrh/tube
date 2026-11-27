# Banc de mesure de Tube en mode test : fenetre invisible, muet, priorite basse, donnees a part
# (%TEMP%\tube-test). Ne lit jamais cookies.txt. Usage :
#   powershell -File scripts\banc.ps1 -Exe <tube.exe> -Nom avant [-Largeur 5120 -Hauteur 1392]
# Mesures (lignes du journal) :
#   -  video X : lecture N ms apres le clic  : demarrage a froid (lancement avec un lien),
#     puis a chaud (clic sur un resultat precharge, puis sur un resultat lointain) ;
#   -  page K : gardee / a charger  puis  complete en N ms  : retour sur la page ;
#   -  interface : ... ms en moyenne, pire ... ms  : defilement continu de la grille.
param([Parameter(Mandatory)][string]$Exe, [string]$Nom = 'essai', [int]$Largeur = 2560, [int]$Hauteur = 1392,
      [string]$Video = 'dQw4w9WgXcQ', [string]$Recherche = 'musique 80s')
$ErrorActionPreference = 'Stop'
$T = Join-Path $env:TEMP 'tube-test'
New-Item -ItemType Directory -Force $T | Out-Null
$copie = Join-Path $env:TEMP "TubeBanc-$Nom.exe"
Copy-Item $Exe $copie -Force
# taille de fenetre voulue (reglages du mode test)
$sp = Join-Path $T 'settings.json'
$s = if (Test-Path $sp) { Get-Content $sp -Raw -Encoding UTF8 | ConvertFrom-Json } else { [pscustomobject]@{} }
$s | Add-Member -Force window @($Largeur, $Hauteur)
[IO.File]::WriteAllText($sp, ($s | ConvertTo-Json -Depth 10), (New-Object Text.UTF8Encoding($false)))

function Lancer($argument, $script, $quit) {
    Remove-Item (Join-Path $T 'tube.log') -ErrorAction SilentlyContinue
    $env:TUBE_TEST = '1'; $env:TUBE_QUIT_AFTER = "$quit"; $env:TUBE_SCRIPT = $script; $env:TUBE_LOG_PAINT = '1'
    $p = Start-Process $copie -ArgumentList $argument -PassThru -WindowStyle Hidden
    $p.PriorityClass = 'BelowNormal'
    if (-not $p.WaitForExit(($quit + 40) * 1000)) {
        # ne ferme que la copie de banc (chemin verifie)
        Get-CimInstance Win32_Process -Filter "ProcessId=$($p.Id)" | Where-Object { $_.ExecutablePath -eq $copie } | ForEach-Object { Stop-Process -Id $_.ProcessId -Force }
    }
    Get-Content (Join-Path $T 'tube.log') -Encoding UTF8
}

"===== $Nom ($Largeur x $Hauteur) ====="
"--- 1. demarrage a froid (lancement avec un lien)"
Lancer "https://www.youtube.com/watch?v=$Video" '' 20 | Select-String 'd.marrage$|premi.re image|apr.s le clic|lecture d' | ForEach-Object { $_.Line }
"--- 2. recherche, clic sur le 1er resultat (precharge), retour, defilement, clic lointain"
$sc = '10:play:0;22:back;30:scroll:25;42:scroll:0;44:play:30'
Lancer $Recherche $sc 62 | Select-String 'flux search|apr.s le clic|lecture d|page |interface :|script' | ForEach-Object { $_.Line }
Remove-Item $copie -ErrorAction SilentlyContinue
