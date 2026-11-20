# Remet le drapeau --disable-features=LockProfileCookieDatabase sur les raccourcis d'Opera GX
# (barre des taches, menu Demarrer, Bureau) et sur son lancement automatique (cle Run), par
# exemple apres une mise a jour d'Opera qui les a recrees. Tube peut alors relire les cookies
# d'Opera sans le couper. Idempotent : rien n'est modifie si le drapeau est deja la ; chaque
# raccourci ou valeur est sauvegarde avant modification (dossier sauvegarde-raccourcis-opera).
#
# Le travail est fait par Tube lui-meme (meme code que le bouton du bandeau) : la commande est
# transmise a Tube s'il tourne deja, sinon Tube demarre et l'applique.
param([string]$Tube = (Join-Path $PSScriptRoot '..\dist\Tube.exe'))
if (-not (Test-Path $Tube)) { Write-Error "Tube.exe introuvable : $Tube (parametre -Tube)"; exit 1 }
Start-Process -FilePath $Tube -ArgumentList '--opera-drapeau' -WindowStyle Hidden
"Demande envoyee a Tube. Compte rendu dans %LOCALAPPDATA%\Tube\tube.log (lignes 'opera :')."
